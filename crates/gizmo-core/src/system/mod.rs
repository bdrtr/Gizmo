use crate::world::World;
use std::any::TypeId;

// ==============================================================
// ACCESS INFO (DAG DEPENDENCY GRAPH)
// ==============================================================

#[derive(Default, Clone)]
pub struct AccessInfo {
    pub component_reads: Vec<TypeId>,
    pub component_writes: Vec<TypeId>,
    pub resource_reads: Vec<TypeId>,
    pub resource_writes: Vec<TypeId>,
    pub is_exclusive: bool,
}

impl AccessInfo {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_compatible_with(&self, other: &AccessInfo) -> bool {
        if self.is_exclusive || other.is_exclusive {
            return false;
        }

        for w in &self.component_writes {
            if other.component_writes.contains(w) || other.component_reads.contains(w) {
                return false;
            }
        }
        for r in &self.component_reads {
            if other.component_writes.contains(r) {
                return false;
            }
        }

        for w in &self.resource_writes {
            if other.resource_writes.contains(w) || other.resource_reads.contains(w) {
                return false;
            }
        }
        for r in &self.resource_reads {
            if other.resource_writes.contains(r) {
                return false;
            }
        }

        true
    }
}

// ==============================================================
// PHASE (SYSTEM SET GROUPING)
// ==============================================================

/// Fizik motoru tarzı faz sıralaması.
/// Sistemler bir faza atanır ve fazlar sabit sırada çalışır:
/// `PreUpdate → Update → Physics → PostUpdate → Render`
///
/// Aynı faz içindeki sistemler DAG batching ile paralel çalıştırılır.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum Phase {
    /// Input polling, zaman güncellemesi, olay temizliği
    PreUpdate = 0,
    /// Oyun mantığı, AI, scripting
    #[default]
    Update = 1,
    /// Fizik simülasyonu (fixed timestep ile)
    Physics = 2,
    /// Transform propagation, cleanup
    PostUpdate = 3,
    /// Rendering hazırlığı
    Render = 4,
}

impl Phase {
    /// Tüm fazları sıralı olarak döndürür.
    pub const ALL: [Phase; 5] = [
        Phase::PreUpdate,
        Phase::Update,
        Phase::Physics,
        Phase::PostUpdate,
        Phase::Render,
    ];

    /// Faz adını döndürür (tracing span'ları için).
    pub const fn name(&self) -> &'static str {
        match self {
            Phase::PreUpdate => "pre_update",
            Phase::Update => "update",
            Phase::Physics => "physics",
            Phase::PostUpdate => "post_update",
            Phase::Render => "render",
        }
    }
}

// ==============================================================
// SYSTEM TRAIT
// ==============================================================

/// Bir sistem: her frame'de çalıştırılabilir mantık birimi.
pub trait System: Send + Sync {
    fn run(&mut self, world: &World, dt: f32);
    fn access_info(&self) -> AccessInfo;
}


// ==============================================================
//  ALT MODÜLLER (god-file Tier 3 round-2 bölmesi — verbatim)
// ==============================================================

mod condition;
mod config;
mod into_system;
mod params;
mod schedule;

pub use condition::*;
pub use config::*;
pub use into_system::*;
pub use params::*;
pub use schedule::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    // --- Mock Bileşen ve Kaynaklar ---
    struct CompA;
    struct CompB;

    // Testlerin çalışma sırasını takip etmek için kullanacağımız log
    #[derive(Clone)]
    struct RunLog {
        log: Arc<Mutex<Vec<&'static str>>>,
    }

    impl RunLog {
        fn new() -> Self {
            Self {
                log: Arc::new(Mutex::new(Vec::new())),
            }
        }
        fn push(&self, msg: &'static str) {
            self.log.lock().unwrap().push(msg);
        }
        fn get(&self) -> Vec<&'static str> {
            self.log.lock().unwrap().clone()
        }
    }

    // Basit bir test sistemi oluşturucu
    fn create_system(name: &'static str, log: RunLog) -> impl FnMut() + Send + Sync + 'static {
        move || {
            log.push(name);
        }
    }

    #[test]
    fn test_schedule_access_info_compatibility() {
        let mut info1 = AccessInfo::new();
        info1.component_reads.push(TypeId::of::<CompA>());

        let mut info2 = AccessInfo::new();
        info2.component_reads.push(TypeId::of::<CompA>());

        // İki sistem de sadece OKUYOR, birbiriyle uyumlu (parallel çalışabilir)
        assert!(info1.is_compatible_with(&info2));

        let mut info3 = AccessInfo::new();
        info3.component_writes.push(TypeId::of::<CompA>());

        // Biri okuyor diğeri YAZIYOR, uyumsuz (farklı batch'lerde olmalı)
        assert!(!info1.is_compatible_with(&info3));

        // İkisi de YAZIYOR, uyumsuz
        let mut info4 = AccessInfo::new();
        info4.component_writes.push(TypeId::of::<CompA>());
        assert!(!info3.is_compatible_with(&info4));
    }

    #[test]
    fn test_schedule_dag_batching_independent() {
        let mut schedule = Schedule::new();
        let log = RunLog::new();

        // 3 bağımsız sistem, read/write çakışması yok. Tek bir batch içinde çalışmalı.
        schedule.add_di_system(create_system("sys1", log.clone()));
        schedule.add_di_system(create_system("sys2", log.clone()));
        schedule.add_di_system(create_system("sys3", log.clone()));

        schedule.build();

        // Hepsi aynı anda paralel çalışabileceği için 1 adet batch oluşmalı
        assert_eq!(schedule.legacy_batches.len(), 1);
        assert_eq!(schedule.legacy_batches[0].systems.len(), 3);
    }

    struct PhysicsSet;
    impl SystemSet for PhysicsSet {}

    #[test]
    fn test_system_set_configuration() {
        let mut schedule = Schedule::new();
        let log = RunLog::new();

        schedule.add_di_system(
            create_system("sys_a", log.clone()).in_set::<PhysicsSet>()
        );
        schedule.add_di_system(
            create_system("sys_b", log.clone()).after_set::<PhysicsSet>()
        );

        schedule.configure_set(SetConfig::new::<PhysicsSet>());

        schedule.build();
        
        assert_eq!(schedule.legacy_batches.len(), 2);
    }

    #[test]
    fn test_schedule_dag_batching_with_conflicts() {
        let mut schedule = Schedule::new();
        let log = RunLog::new();

        // sys1: CompA yazıyor
        schedule.add_di_system(create_system("sys1", log.clone()).writes::<CompA>());
        // sys2: CompA okuyor (sys1 ile çakışır, ayrı batch'e gitmeli)
        schedule.add_di_system(create_system("sys2", log.clone()).reads::<CompA>());
        // sys3: CompB yazıyor (hiçbiriyle çakışmaz, sys1 ile aynı batch'e girebilir)
        schedule.add_di_system(create_system("sys3", log.clone()).writes::<CompB>());
        // sys4: CompA yazıyor (sys1 ve sys2 ile çakışır, en sona kalmalı)
        schedule.add_di_system(create_system("sys4", log.clone()).writes::<CompA>());

        schedule.build();

        // Beklenen Batch'ler (Greedy Backward Scan):
        // Batch 0: sys1 (writes CompA)
        // Batch 1: sys2 (reads CompA), sys3 (writes CompB)
        // Batch 2: sys4 (writes CompA)
        assert_eq!(schedule.legacy_batches.len(), 3);
        assert_eq!(schedule.legacy_batches[0].systems.len(), 1);
        assert_eq!(schedule.legacy_batches[1].systems.len(), 2);
        assert_eq!(schedule.legacy_batches[2].systems.len(), 1);
    }

    #[test]
    fn test_schedule_explicit_ordering_before_after() {
        let mut schedule = Schedule::new();
        let log = RunLog::new();

        // sys1 "after" sys2 olarak işaretlendi
        schedule.add_di_system(
            create_system("sys1", log.clone())
                .label("System1")
                .after("System2"),
        );

        schedule.add_di_system(create_system("sys2", log.clone()).label("System2"));

        // sys3 "before" sys2 olarak işaretlendi
        schedule.add_di_system(
            create_system("sys3", log.clone())
                .label("System3")
                .before("System2"),
        );

        schedule.build();

        // Bağımsız olsalar bile (okuma/yazma çakışması olmasa dahi) explicit order yüzünden:
        // Sıralama: sys3 -> sys2 -> sys1 olmalı ve farklı batch'lerde olmalılar
        assert_eq!(schedule.legacy_batches.len(), 3);

        let mut world = World::new();
        schedule.run(&mut world, 0.1);

        let result = log.get();
        assert_eq!(result, vec!["sys3", "sys2", "sys1"]);
    }

    #[test]
    #[should_panic(expected = "Cyclic dependency detected!")]
    fn test_schedule_cyclic_dependency_panics() {
        let mut schedule = Schedule::new();
        let log = RunLog::new();

        schedule.add_di_system(create_system("sysA", log.clone()).label("A").before("B"));

        schedule.add_di_system(create_system("sysB", log.clone()).label("B").before("C"));

        schedule.add_di_system(
            create_system("sysC", log.clone()).label("C").before("A"), // Cycle: A -> B -> C -> A
        );

        // Bu çağrı panic atmalı
        schedule.build();
    }

    #[test]
    fn test_schedule_phase_ordering() {
        let mut schedule = Schedule::new();
        let log = RunLog::new();

        // 3 sistem farklı fazlara atanmış — veri çakışması yok ama
        // faz sıralaması garanti edilmeli: PreUpdate → Physics → Render
        schedule.add_di_system(create_system("render_sys", log.clone()).in_phase(Phase::Render));
        schedule.add_di_system(create_system("physics_sys", log.clone()).in_phase(Phase::Physics));
        schedule
            .add_di_system(create_system("pre_update_sys", log.clone()).in_phase(Phase::PreUpdate));

        schedule.build();

        // Phase modunda olmalı
        assert!(schedule.uses_phases);
        // 3 faz grubu oluşmalı
        assert_eq!(schedule.phase_batches.len(), 3);
        // Sıralama: PreUpdate(0) < Physics(2) < Render(4)
        assert_eq!(schedule.phase_batches[0].0, Phase::PreUpdate);
        assert_eq!(schedule.phase_batches[1].0, Phase::Physics);
        assert_eq!(schedule.phase_batches[2].0, Phase::Render);

        let mut world = World::new();
        schedule.run(&mut world, 0.016);

        // Çalışma sırası deterministik olmalı
        let result = log.get();
        assert_eq!(result, vec!["pre_update_sys", "physics_sys", "render_sys"]);
    }

    #[test]
    fn test_schedule_phase_with_intra_phase_batching() {
        let mut schedule = Schedule::new();
        let log = RunLog::new();

        // Physics fazında 2 çakışan sistem + 1 bağımsız sistem
        schedule.add_di_system(
            create_system("phys1", log.clone())
                .in_phase(Phase::Physics)
                .writes::<CompA>(),
        );
        schedule.add_di_system(
            create_system("phys2", log.clone())
                .in_phase(Phase::Physics)
                .reads::<CompA>(),
        );
        // Update fazında 1 bağımsız sistem
        schedule.add_di_system(create_system("update_sys", log.clone()).in_phase(Phase::Update));

        schedule.build();

        assert!(schedule.uses_phases);
        // 2 faz grubu: Update ve Physics
        assert_eq!(schedule.phase_batches.len(), 2);
        assert_eq!(schedule.phase_batches[0].0, Phase::Update);
        assert_eq!(schedule.phase_batches[1].0, Phase::Physics);

        // Physics fazı 2 batch'e ayrılmalı (writes/reads çakışması)
        assert_eq!(schedule.phase_batches[1].1.len(), 2);

        // Toplam batch sayısı: Update(1) + Physics(2) = 3
        assert_eq!(schedule.total_batch_count(), 3);
    }

    #[test]
    fn write_write_conflict() {
        let mut a = AccessInfo::new();
        a.component_writes.push(TypeId::of::<CompA>());
        let mut b = AccessInfo::new();
        b.component_writes.push(TypeId::of::<CompA>());
        assert!(!a.is_compatible_with(&b));
    }

    #[test]
    fn read_write_conflict() {
        let mut a = AccessInfo::new();
        a.component_reads.push(TypeId::of::<CompA>());
        let mut b = AccessInfo::new();
        b.component_writes.push(TypeId::of::<CompA>());
        assert!(!a.is_compatible_with(&b));
    }

    #[test]
    fn read_read_no_conflict() {
        let mut a = AccessInfo::new();
        a.component_reads.push(TypeId::of::<CompA>());
        let mut b = AccessInfo::new();
        b.component_reads.push(TypeId::of::<CompA>());
        assert!(a.is_compatible_with(&b));
    }

    #[test]
    fn different_types_no_conflict() {
        let mut a = AccessInfo::new();
        a.component_writes.push(TypeId::of::<CompA>());
        let mut b = AccessInfo::new();
        b.component_writes.push(TypeId::of::<CompB>());
        assert!(a.is_compatible_with(&b));
    }

    // REGRESYON (audit 2026-06-29): `Changed<T>`/`Added<T>` filtreleri T'nin
    // `ComponentTicks` belleğini OKUR; aynı bellek `Mut<T>`'nin `deref_mut`'unda
    // YAZILIR. Eskiden `check_aliasing` HİÇBİR erişim bildirmediğinden zamanlayıcı bir
    // `Query<Changed<T>>` sistemini bir `Query<Mut<T>>` yazıcısıyla aynı paralel batch'e
    // koyabiliyordu (is_compatible_with == true) → ticks üzerinde data race / UB.
    #[test]
    fn changed_and_added_declare_read_conflicting_with_mut_writer() {
        use crate::query::{Added, Changed, Mut, Query};

        #[derive(Clone)]
        struct Pos(#[allow(dead_code)] f32);
        impl crate::component::Component for Pos {}

        let pos = TypeId::of::<Pos>();

        let mut changed_info = AccessInfo::new();
        <Query<'static, Changed<Pos>> as SystemParam>::get_access_info(&mut changed_info);
        assert!(
            changed_info.component_reads.contains(&pos),
            "Changed<Pos> Pos'u READ olarak bildirmeli (ticks'i okuyor)"
        );

        let mut added_info = AccessInfo::new();
        <Query<'static, Added<Pos>> as SystemParam>::get_access_info(&mut added_info);
        assert!(
            added_info.component_reads.contains(&pos),
            "Added<Pos> Pos'u READ olarak bildirmeli"
        );

        let mut mut_info = AccessInfo::new();
        <Query<'static, Mut<Pos>> as SystemParam>::get_access_info(&mut mut_info);
        assert!(mut_info.component_writes.contains(&pos));

        // İkisi de Mut<Pos> yazıcısıyla AYNI paralel batch'e konulamamalı.
        assert!(
            !changed_info.is_compatible_with(&mut_info),
            "Changed<Pos> okuyucu, Mut<Pos> yazıcı ile paralel çalıştırılamaz olmalı (data race)"
        );
        assert!(
            !added_info.is_compatible_with(&mut_info),
            "Added<Pos> okuyucu, Mut<Pos> yazıcı ile paralel çalıştırılamaz olmalı (data race)"
        );
    }

    // REGRESYON (audit round 2): `Or<Changed<A>, Changed<B>>` operandlarının erişimini
    // PROPAGATE etmeli — yoksa Or hiçbir şey bildirmez ve zamanlayıcı onu bir Mut<A>/Mut<B>
    // yazıcısıyla aynı paralel batch'e koyabilir (round-1 data-race sınıfının tekrarı).
    #[test]
    fn or_propagates_operand_access() {
        use crate::query::{Changed, Mut, Or, Query};

        #[derive(Clone)]
        struct A(#[allow(dead_code)] f32);
        impl crate::component::Component for A {}
        #[derive(Clone)]
        struct B(#[allow(dead_code)] f32);
        impl crate::component::Component for B {}

        let mut or_info = AccessInfo::new();
        <Query<'static, Or<Changed<A>, Changed<B>>> as SystemParam>::get_access_info(&mut or_info);
        assert!(
            or_info.component_reads.contains(&TypeId::of::<A>())
                && or_info.component_reads.contains(&TypeId::of::<B>()),
            "Or<Changed<A>,Changed<B>> hem A hem B'yi READ olarak bildirmeli"
        );

        let mut writer_a = AccessInfo::new();
        <Query<'static, Mut<A>> as SystemParam>::get_access_info(&mut writer_a);
        assert!(
            !or_info.is_compatible_with(&writer_a),
            "Or<Changed<A>,..> okuyucu, Mut<A> yazıcı ile paralel çalışamaz olmalı (data race)"
        );
    }
}
