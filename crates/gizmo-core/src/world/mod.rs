use crate::archetype::index::ArchetypeIndex;
use crate::archetype::{ComponentInfo, EntityLocation};
use crate::entity::Entity;

use std::any::TypeId;
use std::collections::HashMap;
use std::sync::RwLock;

mod component_ops;
mod entity_lifecycle;
mod hierarchy_sort;
pub mod hooks;
mod query;
mod registration;
pub mod resources;

pub use self::hooks::*;
pub use self::resources::*;
pub use crate::entity::allocator::Entities;

pub struct World {
    // Entity'den bağımsız global veriler (Time, WindowSize, Input vs.)
    resources: HashMap<TypeId, RwLock<Box<dyn std::any::Any + Send + Sync>>>,

    /// Entity ID → archetype konumu. Hızlı O(1) lookup sağlar.
    /// entity_id indeks olarak kullanılır.
    entity_locations: Vec<EntityLocation>,

    /// Archetype tabanlı depolama — tüm component verileri burada tutulur.
    pub(crate) archetype_index: ArchetypeIndex,

    /// Runtime component metadata cache'i. Archetype sütunları oluşturmak için gereklidir.
    component_infos: HashMap<TypeId, ComponentInfo>,

    pub(crate) component_hooks: HashMap<TypeId, ComponentHooks>,
    pub(crate) sparse_sets: HashMap<TypeId, crate::archetype::sparse_set::ComponentSparseSet>,

    despawn_hooks: Vec<DespawnHook>,
    entities_to_despawn: Vec<Entity>,
    is_despawning: bool,
    pub(crate) entity_observers: HashMap<TypeId, Box<dyn std::any::Any + Send + Sync>>,
    pub tick: u32,
    /// Değişiklik tespiti (change detection) referans tick'i: `Changed<T>`/`Added<T>`
    /// filtreleri `ticks.changed > change_ref_tick` ile bu değere göre karşılaştırır.
    /// Schedule, her frame başında bunu bir önceki frame'in tick'ine ayarlar; böylece
    /// "son frame'den beri değişenler" doğru raporlanır. (Eskiden `== tick` idi ve tick
    /// hiç ilerlemediği için ya hiçbir şeyi ya da her şeyi eşliyordu.)
    pub change_ref_tick: u32,
}

impl World {
    pub fn new() -> Self {
        let mut world = Self {
            resources: HashMap::new(),
            entity_locations: Vec::new(),
            archetype_index: ArchetypeIndex::new(),
            component_infos: HashMap::new(),
            component_hooks: HashMap::new(),
            sparse_sets: HashMap::new(),
            despawn_hooks: Vec::new(),
            entities_to_despawn: Vec::new(),
            is_despawning: false,
            entity_observers: HashMap::new(),
            tick: 1,
            change_ref_tick: 0,
        };
        world.insert_resource(crate::commands::CommandQueue::new());
        world.insert_resource(Entities::new());
        world.insert_resource(Entities::new());
        world
    }

    fn run_hooks<F>(&mut self, type_id: TypeId, mut f: F)
    where
        F: FnMut(&mut ComponentHooks, &mut World),
    {
        let mut hooks = self.component_hooks.remove(&type_id);
        if let Some(ref mut h) = hooks {
            f(h, self);
        }
        if let Some(h) = hooks {
            if let Some(existing) = self.component_hooks.get_mut(&type_id) {
                existing.on_add.extend(h.on_add);
                existing.on_set.extend(h.on_set);
                existing.on_remove.extend(h.on_remove);
            } else {
                self.component_hooks.insert(type_id, h);
            }
        }
    }

    /// Increments the local tick counter, guaranteeing it skips 0 on wrap.
    pub fn increment_tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        if self.tick == 0 {
            self.tick = 1;
        }

        // Apply topological memory alignment for caching locality
        self.sort_archetype_hierarchy();
    }

    /// Frame başında değişiklik-tespiti penceresini açar: bu frame'in karşılaştırma
    /// referansını `ref_tick`'e (bir önceki çalıştırmanın tick'i) ayarlar ve dünya
    /// tick'ini bu frame için ilerletir. `Changed<T>`/`Added<T>` filtreleri
    /// `ticks.changed > change_ref_tick` ile karşılaştırır. Yeni tick'i döndürür.
    /// (Sort yan-etkisi olan `increment_tick`'ten farklı olarak yalnızca sayaç ilerler.)
    pub fn begin_change_frame(&mut self, ref_tick: u32) -> u32 {
        self.change_ref_tick = ref_tick;
        self.tick = self.tick.wrapping_add(1);
        if self.tick == 0 {
            self.tick = 1;
        }
        self.tick
    }

    /// Ertelenmiş komut kuyruğunu (CommandQueue) işler.
    /// Entity ekleme/çıkarma işlemleri bu sayede kilitlenme (deadlock) yaşamadan batch halinde uygulanır.
    pub fn apply_commands(&mut self) {
        let queue_opt = self
            .get_resource::<crate::commands::CommandQueue>()
            .map(|q| (*q).clone());
        if let Some(queue) = queue_opt {
            queue.apply(self);
        }
    }
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::Children;

    #[derive(Clone, PartialEq, Debug)]
    struct Transform(f32);
    impl crate::component::Component for Transform {}

    #[test]
    fn test_sort_archetype_hierarchy() {
        let mut world = World::new();

        // 5 entity oluşturalım: e0, e1, e2, e3, e4
        let e0 = world.spawn();
        let e1 = world.spawn();
        let e2 = world.spawn();
        let e3 = world.spawn();
        let e4 = world.spawn();

        // Hepsi aynı bileşenlere sahip olsun (aynı archetype'a girmeleri için)
        // Sırasıyla Transform ekliyoruz:
        world.add_component(e0, Transform(0.0));
        world.add_component(e1, Transform(1.0));
        world.add_component(e2, Transform(2.0));
        world.add_component(e3, Transform(3.0));
        world.add_component(e4, Transform(4.0));

        // Hiyerarşi kuralım: e0'ın çocukları e3 ve e4 olsun.
        // Başlangıçta e0(0), e1(1), e2(2), e3(3), e4(4) sırasıyla dizilidir.
        world.add_component(e0, Children(vec![e3.id(), e4.id()]));

        // Sadece e0'da Children olunca farklı archetype'a geçer (Archetype değişimi).
        // Bu yüzden hepsine Children eklemeliyiz ki AYNI archetype'da kalsınlar.
        world.add_component(e1, Children(vec![]));
        world.add_component(e2, Children(vec![]));
        world.add_component(e3, Children(vec![]));
        world.add_component(e4, Children(vec![]));

        // Şu an hepsi (Transform, Children) archetype'ında.
        // Beklenen indeksler: e0, e1, e2, e3, e4.

        // Hiyerarşi kaydırmasını çalıştır!
        world.sort_archetype_hierarchy();

        // Kontrol edelim. e0'dan hemen sonra e3 ve e4 gelmeli.
        let loc0 = world.entity_location(e0.id());
        let loc3 = world.entity_location(e3.id());
        let loc4 = world.entity_location(e4.id());

        assert_eq!(
            loc0.row + 1,
            loc3.row,
            "e3 (child), e0 (parent)'dan hemen sonra gelmeli"
        );
        assert_eq!(
            loc0.row + 2,
            loc4.row,
            "e4 (child), e3'ten hemen sonra gelmeli"
        );

        // Diğerleri (e1 ve e2) kaydırılmış olmalı.
        let loc1 = world.entity_location(e1.id());
        let loc2 = world.entity_location(e2.id());
        assert!(
            loc1.row > loc4.row || loc2.row > loc4.row,
            "Bağımsız entityler sona itilmeli"
        );
    }

    #[test]
    fn test_sort_archetype_hierarchy_deep() {
        let mut world = World::new();

        let e0 = world.spawn();
        let e1 = world.spawn();
        let e2 = world.spawn();
        let e3 = world.spawn();

        world.add_component(e0, Transform(0.0));
        world.add_component(e1, Transform(1.0));
        world.add_component(e2, Transform(2.0));
        world.add_component(e3, Transform(3.0));

        // e0 -> e1 -> e2 -> e3 zinciri
        world.add_component(e0, Children(vec![e1.id()]));
        world.add_component(e1, Children(vec![e2.id()]));
        world.add_component(e2, Children(vec![e3.id()]));
        world.add_component(e3, Children(vec![]));

        world.sort_archetype_hierarchy();

        let l0 = world.entity_location(e0.id());
        let l1 = world.entity_location(e1.id());
        let l2 = world.entity_location(e2.id());
        let l3 = world.entity_location(e3.id());

        assert_eq!(l0.row + 1, l1.row);
        // Not: Algoritma şu an sadece doğrudan çocukları hemen arkasına koyar.
        // e1 işlendiğinde e2 onun arkasına geçer, e2 işlendiğinde e3 onun arkasına geçer.
        // Sonuçta e0, e1, e2, e3 dizilimi kendiliğinden oluşur (visited mantığı).
        assert_eq!(l1.row + 1, l2.row);
        assert_eq!(l2.row + 1, l3.row);
    }


    #[test]
    fn spawn_despawn_generation() {
        let mut world = World::new();
        let e1 = world.spawn();
        world.despawn(e1);
        
        let e2 = world.spawn(); // aynı id, farklı generation
        assert_eq!(e1.id(), e2.id());
        assert_ne!(e1.generation(), e2.generation());
        
        // Eski handle artık geçersiz
        assert!(!world.is_alive(e1));
        assert!(world.is_alive(e2));
    }

    #[test]
    fn despawn_updates_swapped_entity_location() {
        #[derive(Clone)]
        struct TestComp(i32);
        impl crate::component::Component for TestComp {}

        let mut world = World::new();
        world.register_component_type::<TestComp>();
        
        let e1 = world.spawn(); world.add_component(e1, TestComp(1));
        let e2 = world.spawn(); world.add_component(e2, TestComp(2));
        let e3 = world.spawn(); world.add_component(e3, TestComp(3));
        
        // e2'yi despawn et — e3 onun yerine swap_remove ile gelir
        world.despawn(e2);
        
        // e3 hâlâ erişilebilir olmalı
        let comps = world.borrow::<TestComp>();
        let val = comps.get(e3.id()).unwrap();
        assert_eq!(val.0, 3);
    }

    #[test]
    fn add_component_migrates_archetype() {
        #[derive(Clone, Debug, PartialEq)]
        struct TestCompI32(i32);
        impl crate::component::Component for TestCompI32 {}

        #[derive(Clone, Debug, PartialEq)]
        struct TestCompF32(f32);
        impl crate::component::Component for TestCompF32 {}

        let mut world = World::new();
        world.register_component_type::<TestCompI32>();
        world.register_component_type::<TestCompF32>();
        
        let e = world.spawn();
        world.add_component(e, TestCompI32(10));
        
        let loc1 = world.entity_location(e.id());
        
        world.add_component(e, TestCompF32(2.5));
        
        let loc2 = world.entity_location(e.id());
        assert_ne!(loc1.archetype_id, loc2.archetype_id);
        
        assert_eq!(world.borrow::<TestCompI32>().get(e.id()).unwrap().0, 10);
        assert_eq!(world.borrow::<TestCompF32>().get(e.id()).unwrap().0, 2.5);
    }

    #[test]
    fn spawn_batch_keeps_columns_and_entities_consistent() {
        #[derive(Clone, Debug, PartialEq)]
        struct BatchI(i32);
        impl crate::component::Component for BatchI {}
        #[derive(Clone, Debug, PartialEq)]
        struct BatchF(f32);
        impl crate::component::Component for BatchF {}

        let mut world = World::new();
        world.register_component_type::<BatchI>();
        world.register_component_type::<BatchF>();

        let n = 100usize;
        let bundles = (0..n).map(|i| (BatchI(i as i32), BatchF(i as f32 * 1.5)));
        let ents: Vec<_> = world.spawn_batch(bundles).collect();
        assert_eq!(ents.len(), n);

        // Her entity'nin iki bileşeni de doğru olmalı (column/entities desync veya OOB yok).
        let bi = world.borrow::<BatchI>();
        let bf = world.borrow::<BatchF>();
        for (i, e) in ents.iter().enumerate() {
            assert_eq!(bi.get(e.id()).map(|c| c.0), Some(i as i32), "BatchI[{i}]");
            assert_eq!(bf.get(e.id()).map(|c| c.0), Some(i as f32 * 1.5), "BatchF[{i}]");
        }
        // Query iterasyonu tam n eleman vermeli (her sütun uzunluğu == entities sayısı).
        assert_eq!(bi.iter().count(), n, "column/entities tutarsızlığı");
        assert_eq!(bf.iter().count(), n, "column/entities tutarsızlığı");
    }

    #[test]
    fn add_same_component_overwrites() {
        #[derive(Clone, Debug, PartialEq)]
        struct TestCompI32(i32);
        impl crate::component::Component for TestCompI32 {}

        let mut world = World::new();
        world.register_component_type::<TestCompI32>();
        
        let e = world.spawn();
        world.add_component(e, TestCompI32(1));
        world.add_component(e, TestCompI32(99)); // overwrite
        
        assert_eq!(world.borrow::<TestCompI32>().get(e.id()).unwrap().0, 99);
    }

    #[test]
    fn archetype_graph_reuses_archetypes() {
        #[derive(Clone, Debug, PartialEq)]
        struct TestCompI32(i32);
        impl crate::component::Component for TestCompI32 {}

        #[derive(Clone, Debug, PartialEq)]
        struct TestCompF32(f32);
        impl crate::component::Component for TestCompF32 {}

        let mut world = World::new();
        world.register_component_type::<TestCompI32>();
        world.register_component_type::<TestCompF32>();
        
        let e1 = world.spawn(); world.add_component(e1, TestCompI32(1)); world.add_component(e1, TestCompF32(1.0));
        let e2 = world.spawn(); world.add_component(e2, TestCompI32(2)); world.add_component(e2, TestCompF32(2.0));
        
        let loc1 = world.entity_location(e1.id());
        let loc2 = world.entity_location(e2.id());
        assert_eq!(loc1.archetype_id, loc2.archetype_id);
        
        assert!(world.archetype_index.archetypes.len() < 5);
    }

    #[test]
    fn query_finds_matching_archetypes() {
        #[derive(Clone)]
        #[allow(dead_code)]
        struct TestCompI32(i32);
        impl crate::component::Component for TestCompI32 {}

        #[derive(Clone)]
        #[allow(dead_code)]
        struct TestCompF32(f32);
        impl crate::component::Component for TestCompF32 {}

        #[derive(Clone)]
        #[allow(dead_code)]
        struct TestCompBool(bool);
        impl crate::component::Component for TestCompBool {}

        let mut world = World::new();
        world.register_component_type::<TestCompI32>();
        world.register_component_type::<TestCompF32>();
        world.register_component_type::<TestCompBool>();
        
        let e1 = world.spawn(); world.add_component(e1, TestCompI32(1)); world.add_component(e1, TestCompF32(1.0));
        let e2 = world.spawn(); world.add_component(e2, TestCompI32(2)); world.add_component(e2, TestCompBool(true));
        let e3 = world.spawn(); world.add_component(e3, TestCompI32(3)); // sadece i32
        
        // i32 query'si 3 entity'yi de bulmalı
        let count = world.query::<&TestCompI32>().unwrap().iter().count();
        assert_eq!(count, 3);
        
        // (i32, f32) query'si sadece e1'i bulmalı
        let count = world.query::<(&TestCompI32, &TestCompF32)>().unwrap().iter().count();
        assert_eq!(count, 1);
    }

    #[test]
    fn query_mut_modifies_data() {
        #[derive(Clone)]
        struct TestCompI32(i32);
        impl crate::component::Component for TestCompI32 {}

        let mut world = World::new();
        world.register_component_type::<TestCompI32>();
        
        let e1 = world.spawn(); world.add_component(e1, TestCompI32(1));
        let e2 = world.spawn(); world.add_component(e2, TestCompI32(2));
        
        // Query ile tüm i32'leri iki katına çıkar
        if let Some(mut q) = world.query_mut::<crate::query::Mut<TestCompI32>>() {
            for (_, mut val) in q.iter_mut() {
                val.0 *= 2;
            }
        }
        
        assert_eq!(world.borrow::<TestCompI32>().get(e1.id()).unwrap().0, 2);
        assert_eq!(world.borrow::<TestCompI32>().get(e2.id()).unwrap().0, 4);
    }

    #[test]
    fn query_skips_non_matching() {
        #[derive(Clone)]
        struct CompA;
        impl crate::component::Component for CompA {}
        #[derive(Clone)]
        struct CompB;
        impl crate::component::Component for CompB {}

        let mut world = World::new();
        world.register_component_type::<CompA>();
        world.register_component_type::<CompB>();

        for _ in 0..100 {
            let e = world.spawn();
            world.add_component(e, CompA);
        }

        for _ in 0..50 {
            let e = world.spawn();
            world.add_component(e, CompB);
        }

        let a_count = world.query::<&CompA>().unwrap().iter().count();
        let b_count = world.query::<&CompB>().unwrap().iter().count();
        let both_count = world.query::<(&CompA, &CompB)>().unwrap().iter().count();

        assert_eq!(a_count, 100);
        assert_eq!(b_count, 50);
        assert_eq!(both_count, 0);
    }

    #[test]
    fn spawn_despawn_10k_entities_archetype_stability() {
        #[derive(Clone)]
        #[allow(dead_code)]
        struct CompA(i32);
        impl crate::component::Component for CompA {}
        #[derive(Clone)]
        #[allow(dead_code)]
        struct CompB(f32);
        impl crate::component::Component for CompB {}

        let mut world = World::new();
        world.register_component_type::<CompA>();
        world.register_component_type::<CompB>();

        let initial_archetypes = world.archetype_index.archetypes.len();

        // Spawn 10k entities
        let mut entities = Vec::new();
        for i in 0..10_000 {
            let e = world.spawn();
            world.add_component(e, CompA(i));
            if i % 2 == 0 {
                world.add_component(e, CompB(i as f32));
            }
            entities.push(e);
        }

        // Despawn all
        for e in entities {
            world.despawn(e);
        }

        // Archetype sayısı aynı kalmalı
        let final_archetypes = world.archetype_index.archetypes.len();
        // 1 empty, 1 for CompA, 1 for (CompA, CompB) = 3 total usually.
        assert!(final_archetypes <= initial_archetypes + 2);
    }
}
