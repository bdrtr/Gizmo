/// ECS Entity tanımlayıcısı — Packed u64 temsili.
///
/// Alt 32 bit = entity ID (slot index), üst 32 bit = generation (yeniden kullanım sayacı).
/// Generation, aynı slot'a (ID'ye) yeni bir entity atandığında artırılır ve eski referansların
/// (dangling entity) güvenli şekilde tespit edilmesini sağlar.
///
/// # Layout
/// ```text
/// ┌──────────────────────────────────────────────────────────────────┐
/// │  63 ───────────── 32 │ 31 ───────────── 0 │
/// │     generation (u32) │       id (u32)     │
/// └──────────────────────────────────────────────────────────────────┘
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Entity(u64);

impl Entity {
    /// Geçersiz / null entity sentinel değeri.
    /// `Option<Entity>` yerine kullanılabilir (ergonomi ve cache dostu).
    pub const INVALID: Self = Self(u64::MAX);

    /// Yeni bir Entity oluşturur.
    #[inline]
    pub fn new(id: u32, generation: u32) -> Self {
        Self(((generation as u64) << 32) | id as u64)
    }

    /// Entity'nin slot indeksini (ID) döndürür.
    #[inline]
    pub fn id(self) -> u32 {
        self.0 as u32
    }

    /// Entity'nin generation (nesil) sayacını döndürür.
    #[inline]
    pub fn generation(self) -> u32 {
        (self.0 >> 32) as u32
    }

    /// Bu entity'nin geçerli (INVALID olmayan) olup olmadığını kontrol eder.
    #[inline]
    pub fn is_valid(self) -> bool {
        self != Self::INVALID
    }

    /// Entity'yi ham u64 bit temsiline dönüştürür.
    /// Serializasyon, network sync ve hash key olarak kullanılabilir.
    #[inline]
    pub fn to_bits(self) -> u64 {
        self.0
    }

    /// Ham u64 bit temsilinden Entity oluşturur.
    #[inline]
    pub fn from_bits(bits: u64) -> Self {
        Self(bits)
    }
}

impl std::fmt::Display for Entity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if *self == Self::INVALID {
            write!(f, "Entity(INVALID)")
        } else {
            write!(f, "Entity({}:{})", self.id(), self.generation())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_and_accessors() {
        let e = Entity::new(42, 7);
        assert_eq!(e.id(), 42);
        assert_eq!(e.generation(), 7);
    }

    #[test]
    fn test_zero_generation() {
        let e = Entity::new(0, 0);
        assert_eq!(e.id(), 0);
        assert_eq!(e.generation(), 0);
        assert!(e.is_valid());
    }

    #[test]
    fn test_max_values() {
        let e = Entity::new(u32::MAX - 1, u32::MAX - 1);
        assert_eq!(e.id(), u32::MAX - 1);
        assert_eq!(e.generation(), u32::MAX - 1);
        assert!(e.is_valid());
    }

    #[test]
    fn test_invalid_sentinel() {
        assert!(!Entity::INVALID.is_valid());
        assert_eq!(Entity::INVALID.id(), u32::MAX);
        assert_eq!(Entity::INVALID.generation(), u32::MAX);
    }

    #[test]
    fn test_to_bits_from_bits_roundtrip() {
        let e = Entity::new(123, 456);
        let bits = e.to_bits();
        let e2 = Entity::from_bits(bits);
        assert_eq!(e, e2);
        assert_eq!(e2.id(), 123);
        assert_eq!(e2.generation(), 456);
    }

    #[test]
    fn test_display() {
        let e = Entity::new(5, 2);
        assert_eq!(format!("{}", e), "Entity(5:2)");
    }

    #[test]
    fn test_display_invalid() {
        assert_eq!(format!("{}", Entity::INVALID), "Entity(INVALID)");
    }

    #[test]
    fn test_equality_and_hash() {
        use std::collections::HashSet;
        let e1 = Entity::new(1, 0);
        let e2 = Entity::new(1, 0);
        let e3 = Entity::new(1, 1); // Aynı ID, farklı generation

        assert_eq!(e1, e2);
        assert_ne!(e1, e3);

        let mut set = HashSet::new();
        set.insert(e1);
        assert!(set.contains(&e2));
        assert!(!set.contains(&e3));
    }

    #[test]
    fn test_copy_semantics() {
        let e1 = Entity::new(10, 3);
        let e2 = e1; // Copy
        assert_eq!(e1, e2);
        assert_eq!(e1.id(), e2.id());
    }

    #[test]
    fn test_serde_roundtrip() {
        let e = Entity::new(999, 42);
        let serialized = ron::to_string(&e).expect("serialize failed");
        let deserialized: Entity = ron::from_str(&serialized).expect("deserialize failed");
        assert_eq!(e, deserialized);
    }
}
pub mod allocator;
