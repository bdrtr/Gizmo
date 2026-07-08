//! Dünya (World) introspection / analiz yüzeyi.
//!
//! Salt-okunur, davranış değiştirmeyen erişimciler. Amaç: dışarıdaki analiz katmanının
//! (gizmo-analysis) çalışan bir motorun ECS durumunun **en ufak ayrıntısına** — hangi
//! archetype'ta hangi component'lerden kaç tane var, kaç bayt tutuyor — erişebilmesi.
//!
//! Bu modül `world` modülünün alt-modülü olduğundan `World`'ün private alanlarına
//! (archetype_index, component_infos, sparse_sets, resources) erişebilir; hiçbirini
//! değiştirmez, yalnız okur.

use super::World;
use std::any::TypeId;

/// Bir archetype içindeki tek bir component tipinin özeti.
#[derive(Debug, Clone)]
pub struct ComponentSummary {
    pub type_id: TypeId,
    /// `std::any::type_name` (kayıt anında yakalanan tam yol).
    pub name: &'static str,
    /// Tek bir instance'ın bayt boyutu (`Layout::size`).
    pub item_size: usize,
    /// Bu archetype'taki instance sayısı (= archetype'ın entity sayısı).
    pub count: usize,
    /// `item_size * count`.
    pub bytes: usize,
}

impl ComponentSummary {
    /// Tip adının son segmenti (`a::b::Transform` → `Transform`). Generic'lerde
    /// baştaki yolu kırpar ama `<...>` içini bırakır.
    pub fn short_name(&self) -> &str {
        short_type_name(self.name)
    }
}

/// Tek bir archetype (aynı component bileşimine sahip entity tablosu) özeti.
#[derive(Debug, Clone)]
pub struct ArchetypeSummary {
    pub id: u32,
    pub entity_count: usize,
    /// Bu archetype'ın tüm component sütunlarının toplam baytı.
    pub bytes: usize,
    /// Component'ler bayt kullanımına göre azalan sırada.
    pub components: Vec<ComponentSummary>,
}

/// Dünyanın üst-düzey sayaçları — tek bakışta durum.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorldStats {
    /// Canlı entity sayısı (tüm archetype'lardaki satırların toplamı).
    pub entities: usize,
    /// Toplam archetype sayısı (boşlar dahil).
    pub archetypes: usize,
    /// İçinde en az bir entity olan archetype sayısı.
    pub non_empty_archetypes: usize,
    /// Kayıtlı (görülmüş) component tipi sayısı.
    pub registered_components: usize,
    /// Sparse-set depolamalı component tipi sayısı.
    pub sparse_set_components: usize,
    /// Kayıtlı resource sayısı.
    pub resources: usize,
    /// Archetype sütunlarındaki toplam component baytı (yaklaşık canlı ECS belleği).
    pub component_bytes: usize,
    /// Dünya tick'i.
    pub tick: u32,
}

/// `a::b::c::Type<x::y::Z>` → `Type<Z>` benzeri kısa ad. Sığ ama pratik.
pub fn short_type_name(full: &str) -> &str {
    // Generic argümanların başlangıcından önceki son `::`'yi bul.
    let head_end = full.find('<').unwrap_or(full.len());
    let head = &full[..head_end];
    match head.rfind("::") {
        Some(pos) => &full[pos + 2..],
        None => full,
    }
}

impl World {
    /// Archetype tablolarındaki toplam canlı satır sayısı. (`World::entity_count`
    /// zaten allocator tarafından tanımlı; bu, depolama tarafından görülen sayıdır —
    /// normalde eşittirler.)
    #[inline]
    pub fn stored_entity_count(&self) -> usize {
        self.archetype_index
            .archetypes
            .iter()
            .map(|a| a.len())
            .sum()
    }

    /// Toplam archetype sayısı.
    #[inline]
    pub fn archetype_count(&self) -> usize {
        self.archetype_index.archetypes.len()
    }

    /// Kayıtlı resource sayısı.
    #[inline]
    pub fn resource_count(&self) -> usize {
        self.resources.len()
    }

    /// Bir component tipinin insan-okunur adı (kayıtlıysa).
    #[inline]
    pub fn component_type_name(&self, type_id: TypeId) -> Option<&'static str> {
        self.component_infos.get(&type_id).map(|i| i.type_name)
    }

    /// Üst-düzey dünya istatistikleri.
    pub fn world_stats(&self) -> WorldStats {
        let mut entities = 0usize;
        let mut non_empty = 0usize;
        let mut component_bytes = 0usize;

        for arch in &self.archetype_index.archetypes {
            let n = arch.len();
            if n == 0 {
                continue;
            }
            non_empty += 1;
            entities += n;
            for type_id in arch.component_types() {
                if let Some(info) = self.component_infos.get(&type_id) {
                    component_bytes += info.layout.size() * n;
                }
            }
        }

        WorldStats {
            entities,
            archetypes: self.archetype_index.archetypes.len(),
            non_empty_archetypes: non_empty,
            registered_components: self.component_infos.len(),
            sparse_set_components: self.sparse_sets.len(),
            resources: self.resources.len(),
            component_bytes,
            tick: self.tick,
        }
    }

    /// Boş olmayan her archetype için ayrıntılı özet (component adları + bayt + sayı).
    /// Sonuç entity sayısına göre azalan sırada.
    pub fn archetype_summaries(&self) -> Vec<ArchetypeSummary> {
        let mut out = Vec::new();

        for arch in &self.archetype_index.archetypes {
            let n = arch.len();
            if n == 0 {
                continue;
            }

            let mut components = Vec::new();
            let mut arch_bytes = 0usize;
            for type_id in arch.component_types() {
                let (name, item_size) = match self.component_infos.get(&type_id) {
                    Some(info) => (info.type_name, info.layout.size()),
                    None => ("<unregistered>", 0),
                };
                let bytes = item_size * n;
                arch_bytes += bytes;
                components.push(ComponentSummary {
                    type_id,
                    name,
                    item_size,
                    count: n,
                    bytes,
                });
            }
            // Bayt kullanımına göre azalan; eşitlikte ada göre deterministik.
            components.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(b.name)));

            out.push(ArchetypeSummary {
                id: arch.id,
                entity_count: n,
                bytes: arch_bytes,
                components,
            });
        }

        out.sort_by(|a, b| b.entity_count.cmp(&a.entity_count).then(a.id.cmp(&b.id)));
        out
    }
}

#[cfg(test)]
mod tests {
    use crate::world::World;

    #[derive(Clone)]
    struct Position {
        _x: f32,
        _y: f32,
        _z: f32,
    }
    #[derive(Clone)]
    struct Velocity {
        _v: [f32; 3],
    }
    impl crate::component::Component for Position {}
    impl crate::component::Component for Velocity {}

    #[test]
    fn introspection_reports_entities_archetypes_and_names() {
        let mut world = World::new();

        let a = world.spawn();
        world.add_component(a, Position { _x: 0.0, _y: 0.0, _z: 0.0 });

        let b = world.spawn();
        world.add_component(b, Position { _x: 1.0, _y: 0.0, _z: 0.0 });
        world.add_component(b, Velocity { _v: [0.0; 3] });

        let stats = world.world_stats();
        assert_eq!(stats.entities, 2);
        assert!(stats.registered_components >= 2);
        assert!(stats.component_bytes >= std::mem::size_of::<Position>() * 2);

        let summaries = world.archetype_summaries();
        // {Position} ve {Position,Velocity} olmak üzere iki dolu archetype.
        assert_eq!(summaries.len(), 2);

        // En az bir archetype Position içermeli ve adı çözülmüş olmalı.
        let has_position = summaries.iter().any(|s| {
            s.components
                .iter()
                .any(|c| c.short_name() == "Position" && c.item_size == std::mem::size_of::<Position>())
        });
        assert!(has_position, "Position component adı/boyutu çözülemedi");

        // Toplam entity, archetype-özet sayımıyla tutarlı.
        let total: usize = summaries.iter().map(|s| s.entity_count).sum();
        assert_eq!(total, 2);
    }

    #[test]
    fn short_type_name_strips_path_keeps_generics_tail() {
        use super::short_type_name;
        assert_eq!(short_type_name("gizmo_physics_core::Transform"), "Transform");
        assert_eq!(short_type_name("Foo"), "Foo");
    }
}
