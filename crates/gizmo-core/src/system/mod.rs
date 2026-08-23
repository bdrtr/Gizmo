//! Systems, their declared access, and the scheduler that batches them.
//!
//! A [`System`] is any function the engine can call with the world and a delta. Each one
//! reports an [`AccessInfo`] — which components and resources it reads and writes — and the
//! scheduler packs mutually compatible systems into batches that run in parallel.
//!
//! Access declarations control BATCHING, not order. Two systems that conflict are merely kept
//! out of the same batch; which of them runs first is the batcher's choice. The only ways a
//! caller picks an order are `before`/`after` labels and [`Phase`].
use crate::world::World;
use std::any::TypeId;

// ==============================================================
// ACCESS INFO (DAG DEPENDENCY GRAPH)
// ==============================================================

/// What a system touches in the world, and whether it must run on its own.
///
/// The scheduler derives one of these per system — from its [`SystemParam`]s, plus anything
/// declared by hand on the [`SystemConfig`] builder — and uses
/// [`is_compatible_with`](AccessInfo::is_compatible_with) to decide which systems may share a
/// parallel batch.
///
/// Under-declaring is a soundness bug, not a performance one: state a system touches without
/// declaring it can be co-scheduled with a writer of that same state and raced on.
/// Over-declaring only costs parallelism, so err that way.
///
/// The four lists are append-only bags of `TypeId`. Entries appear in push order, duplicates
/// are never removed (a batch's info is literally the concatenation of its members'), and
/// neither the order nor the multiplicity carries meaning — do not rely on either.
#[derive(Default, Clone)]
pub struct AccessInfo {
    /// Components read through a shared borrow (`&T`).
    ///
    /// This also covers components whose *change ticks* are merely inspected — a
    /// `Changed<T>`/`Added<T>` filter declares a read of `T` even though it yields no `&T`,
    /// because those ticks are the same memory a `Mut<T>` writer stamps.
    pub component_reads: Vec<TypeId>,
    /// Components written through `Mut<T>`.
    ///
    /// The same `TypeId` may legitimately appear in `component_reads` as well — a merged
    /// batch info routinely holds both — because an `AccessInfo` is never checked against
    /// itself, only against another one.
    pub component_writes: Vec<TypeId>,
    /// Resources read through `Res<T>`.
    ///
    /// Resource and component lists are compared separately, so a type used as a component
    /// and a resource of the same `TypeId` never conflicts across the two categories.
    pub resource_reads: Vec<TypeId>,
    /// Resources written through `ResMut<T>`.
    pub resource_writes: Vec<TypeId>,
    /// When true this system tolerates no company: `is_compatible_with` fails against
    /// *every* other info, even an empty one, so the system ends up alone in its batch and
    /// no later system can join it.
    ///
    /// It is the fallback for systems whose access cannot be introspected — the bare
    /// `FnMut(&World, f32)` form, and the untyped `run_if(|world| ..)` closure — as well as
    /// anything the user marks exclusive explicitly.
    pub is_exclusive: bool,
}

impl AccessInfo {
    /// An access set that touches nothing and is not exclusive: compatible with every other
    /// non-exclusive set. Same value as `Default`; the lists are then filled in by pushing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether these two access sets may run concurrently.
    ///
    /// Incompatible when either side is exclusive (unconditionally — including against a set
    /// that touches nothing), or when the same `TypeId` is written by one side and read or
    /// written by the other. Reads never conflict with reads, components are only checked
    /// against components and resources against resources, and two empty sets are compatible.
    ///
    /// The relation is symmetric, and it is *not* transitive: two systems that are each
    /// compatible with a third can still conflict with one another. Cost is O(n·m) linear
    /// scans — the lists hold a handful of types, so no set structure is used.
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

/// Physics-engine style phase ordering.
///
/// Systems are assigned to a phase and the phases run in order. The five built-in phases are
/// `PreUpdate → Update → Physics → PostUpdate → Render`, and systems within one phase run in
/// parallel with DAG batching.
///
/// # Phases of your own
///
/// [`Phase::User`] carries a position on the same scale the built-ins sit on, so a game can put
/// work *between* two built-in phases rather than only after them:
///
/// | position | |
/// |---|---|
/// | `0 … 999` | before `PreUpdate` |
/// | **1000** | `PreUpdate` |
/// | `1001 … 1999` | between `PreUpdate` and `Update` |
/// | **2000** | `Update` |
/// | `2001 … 2999` | between `Update` and `Physics` |
/// | **3000** | `Physics` |
/// | `3001 … 3999` | between `Physics` and `PostUpdate` |
/// | **4000** | `PostUpdate` |
/// | `4001 … 4999` | between `PostUpdate` and `Render` |
/// | **5000** | `Render` |
/// | `5001 …` | after `Render` |
///
/// ```
/// # use gizmo_core::system::Phase;
/// // "after physics has settled, before transforms propagate"
/// let cleanup = Phase::User(3500);
/// assert!(Phase::Physics < cleanup && cleanup < Phase::PostUpdate);
/// ```
///
/// The ordering is [`Phase::position`], not declaration order, which is why this is a manual
/// `Ord`: a derived one would sort every `User` after every built-in and the table above could
/// not exist.
///
/// # Why the enum is `#[non_exhaustive]`
///
/// So that a sixth built-in phase is not a breaking change. Match on it with a `_` arm, or use
/// [`Phase::position`] when what you want is the order rather than the identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Phase {
    /// Input polling, time update, event cleanup. Position **1000**.
    PreUpdate,
    /// Game logic, AI, scripting. Position **2000**.
    #[default]
    Update,
    /// Physics simulation (with a fixed timestep). Position **3000**.
    Physics,
    /// Transform propagation, cleanup. Position **4000**.
    PostUpdate,
    /// Rendering preparation. Position **5000**.
    Render,
    /// A phase belonging to the game, ordered by its position on the scale above.
    ///
    /// Two systems in `User(n)` with the same `n` share a phase and batch together, exactly as
    /// two systems in `Update` do.
    User(u16),
}

impl PartialOrd for Phase {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Phase {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.position().cmp(&other.position())
    }
}

impl Phase {
    /// Where this phase sits in the frame.
    ///
    /// The built-ins are at round thousands so a [`Phase::User`] can be placed between any two of
    /// them; see the type's docs for the table. This is the *only* thing that decides phase
    /// order — [`Ord`] is written in terms of it.
    #[must_use]
    pub const fn position(&self) -> u32 {
        match self {
            Phase::PreUpdate => 1000,
            Phase::Update => 2000,
            Phase::Physics => 3000,
            Phase::PostUpdate => 4000,
            Phase::Render => 5000,
            Phase::User(n) => *n as u32,
        }
    }

    /// The five **built-in** phases, in order.
    ///
    /// Not every phase a schedule may contain: a [`Phase::User`] is not here and cannot be, since
    /// there are 65 536 of them. Code that wants "every phase in this schedule" must read the
    /// schedule, not this constant.
    pub const ALL: [Phase; 5] = [
        Phase::PreUpdate,
        Phase::Update,
        Phase::Physics,
        Phase::PostUpdate,
        Phase::Render,
    ];

    /// Returns the phase name (for tracing spans).
    ///
    /// Every [`Phase::User`] answers `"user"`; the number that separates them is in
    /// [`Phase::position`]. The return type is `&'static str` so that naming a phase inside the
    /// frame loop cannot allocate, and `User(n)` has no static name to give.
    pub const fn name(&self) -> &'static str {
        match self {
            Phase::PreUpdate => "pre_update",
            Phase::Update => "update",
            Phase::Physics => "physics",
            Phase::PostUpdate => "post_update",
            Phase::Render => "render",
            Phase::User(_) => "user",
        }
    }
}

// ==============================================================
// SYSTEM TRAIT
// ==============================================================

/// A system: a unit of logic that can be run every frame.
pub trait System: Send + Sync {
    /// Executes the system once.
    ///
    /// `world` is shared, never exclusive: every system of a batch is handed the same
    /// `&World`, and on native targets they are driven in parallel by rayon. Mutation
    /// therefore goes through [`SystemParam`]s whose accesses the scheduler proved disjoint
    /// before starting the batch (component writes via `Query<Mut<T>>`, resources via their
    /// lock guards), or is deferred through `Commands` and applied after the batch finishes.
    ///
    /// `dt` is the delta time in seconds that the schedule was stepped with; the `f32`
    /// system parameter yields exactly this value.
    ///
    /// Ordering: assume nothing about the order of the other systems in the same batch — it
    /// is a work-stealing detail (on `wasm32` the same batch happens to run sequentially,
    /// but neither that nor the order it falls into is a contract).
    ///
    /// Only two things let a CALLER choose the order: `before`/`after` labels, and a different
    /// [`Phase`]. A declared access conflict merely keeps the pair out of one batch — which
    /// side then runs first falls out of insertion order and the batcher's packing, and is not
    /// something you asked for. Do not use `reads`/`writes` to sequence two systems.
    fn run(&mut self, world: &World, dt: f32);

    /// The world access this system performs, as the scheduler should see it.
    ///
    /// Called while the schedule is being built (not per frame), so it may recompute rather
    /// than cache. It must return a *superset* of what `run` actually touches: anything left
    /// out can be placed in a batch alongside a writer of that data. Wrapper systems
    /// consequently return the union of the parts they wrap, and any system whose access
    /// cannot be introspected must return `is_exclusive = true` — the conservative answer
    /// that is always sound.
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

    // --- Phase::User: kullanıcının kendi fazı ---

    #[test]
    fn a_user_phase_sorts_by_its_position_not_by_declaration_order() {
        // Türetilmiş bir Ord her User'ı her yerleşik fazın *arkasına* atardı. Manuel Ord'un
        // varlık sebebi tam olarak bu satır: 3500 fiziğin sonrası, PostUpdate'in öncesi.
        assert!(Phase::Physics < Phase::User(3500));
        assert!(Phase::User(3500) < Phase::PostUpdate);

        // Ve ölçeğin iki ucu
        assert!(Phase::User(500) < Phase::PreUpdate);
        assert!(Phase::Render < Phase::User(5001));

        // Ord, position() ile aynı şeyi söylemeli — biri değişip diğeri kalırsa test kırılsın.
        let mut phases = [
            Phase::Render,
            Phase::User(2500),
            Phase::PreUpdate,
            Phase::User(999),
            Phase::Update,
        ];
        phases.sort();
        let positions: Vec<u32> = phases.iter().map(Phase::position).collect();
        assert_eq!(positions, vec![999, 1000, 2000, 2500, 5000]);
    }

    #[test]
    fn a_user_phase_runs_between_the_two_built_ins_it_was_placed_between() {
        let mut schedule = Schedule::new();
        let log = RunLog::new();

        // Ekleme sırası kasten ters: sıra fazdan gelmeli, ekleme sırasından değil.
        schedule.add_di_system(create_system("post", log.clone()).in_phase(Phase::PostUpdate));
        schedule.add_di_system(create_system("mine", log.clone()).in_phase(Phase::User(3500)));
        schedule.add_di_system(create_system("physics", log.clone()).in_phase(Phase::Physics));

        schedule.build();

        assert!(schedule.uses_phases);
        assert_eq!(schedule.phase_batches.len(), 3);
        assert_eq!(schedule.phase_batches[1].0, Phase::User(3500));

        let mut world = World::new();
        schedule.run(&mut world, 0.016);
        assert_eq!(log.get(), vec!["physics", "mine", "post"]);
    }

    #[test]
    fn two_systems_in_the_same_user_phase_share_it_and_different_numbers_do_not() {
        let mut schedule = Schedule::new();
        let log = RunLog::new();

        schedule.add_di_system(create_system("a", log.clone()).in_phase(Phase::User(2500)));
        schedule.add_di_system(create_system("b", log.clone()).in_phase(Phase::User(2500)));
        schedule.add_di_system(create_system("c", log.clone()).in_phase(Phase::User(2600)));

        schedule.build();

        // İki faz grubu: {a, b} ve {c}. Aynı sayı aynı faz demek.
        assert_eq!(schedule.phase_batches.len(), 2);
        assert_eq!(schedule.phase_batches[0].0, Phase::User(2500));
        assert_eq!(schedule.phase_batches[1].0, Phase::User(2600));
    }

    #[test]
    fn a_user_phase_alone_still_switches_the_schedule_into_phase_mode() {
        // has_explicit_phase, `phase != Phase::Update` diye bakıyor. User(2000) Update ile aynı
        // *konumda* ama aynı faz değil — çizelge yine de faz kipine geçmeli.
        let mut schedule = Schedule::new();
        let log = RunLog::new();
        schedule.add_di_system(create_system("only", log.clone()).in_phase(Phase::User(2000)));
        schedule.build();

        assert!(schedule.uses_phases);
        assert_eq!(schedule.phase_batches.len(), 1);
        assert_eq!(schedule.phase_batches[0].0, Phase::User(2000));

        let mut world = World::new();
        schedule.run(&mut world, 0.016);
        assert_eq!(log.get(), vec!["only"]);
    }

    #[test]
    fn all_is_the_built_ins_and_says_so() {
        // ALL bir "her faz" listesi değil; User onun dışında. Bu test o iddiayı kilitliyor,
        // çünkü ALL üzerinde dönen kod User'ı sessizce atlar.
        assert_eq!(Phase::ALL.len(), 5);
        assert!(!Phase::ALL.contains(&Phase::User(2000)));
        let positions: Vec<u32> = Phase::ALL.iter().map(Phase::position).collect();
        assert_eq!(positions, vec![1000, 2000, 3000, 4000, 5000]);
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
