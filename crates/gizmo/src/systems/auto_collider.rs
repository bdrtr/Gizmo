//! Otomatik KUTU-COLLIDER — boyutu `Transform.scale`'den türet (boyutu İKİ KEZ yazma).
//!
//! Bir kutu spawn'larken geliştirici bugüne kadar boyutu iki kez yazmak zorundaydı: bir kez
//! görsel ölçek (`Transform::with_scale(half)`), bir kez de eşleşen collider
//! (`Collider::box_collider(half)`). İkisi ayrışırsa fizik ile görsel sessizce birbirinden
//! kopar. Bu, kullanıcının işaret ettiği "geliştiriciyi düşük-seviye tekrara zorlama"nın tam
//! örneği.
//!
//! Çözüm — [`Spin`](crate::systems::spin) / [`LifetimeSystem`](crate::systems::lifetime)
//! idiomunu izleyen opt-in bir işaret: bir [`AutoBoxCollider`] komponenti (+ yer-tutucu kutu
//! collider) ekle, [`PhysicsPlugin`](crate::plugins::PhysicsPlugin) sistemi otomatik
//! çalıştırsın. [`AutoBoxColliderSystem`] ilk fizik adımından ÖNCE her taze işareti bir kez
//! çözer: `Transform.scale · base` → collider yarı-genişliği, atalet yeniden türetilir, sonra
//! işaret kaldırılır. Boyut yalnız BİR KEZ (Transform.scale olarak) yazılır.
//!
//! `Prefab` için senkron bir kısa-yol da var ([`Prefab::auto_box_collider`](crate::bundles::Prefab::auto_box_collider)):
//! `Prefab::spawn` transform'u zaten elinde tuttuğu için collider'ı spawn anında çözer (bir-frame
//! gecikmesi yok). Her iki yol da aynı saf [`derived_box_half_extents`] yardımcısını kullanır.
//!
//! ```ignore
//! // Ham (Prefab'sız) yol — herhangi bir spawn kanalıyla çalışır:
//! world.spawn_bundle((
//!     Transform::new(pos).with_scale(Vec3::new(4.0, 0.4, 1.0)), // boyut BİR kez
//!     mesh, mat, MeshRenderer::new(),
//!     RigidBodyBundle::dynamic(3.0).with_collider(Collider::box_collider(Vec3::ONE)), // yer-tutucu
//!     AutoBoxCollider::new(),  // ilk fizik adımından önce (4.0,0.4,1.0)'e çözülür
//! ));
//! ```

use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::{BoxShape, Collider, ColliderShape, Transform};
use gizmo_physics_rigid::components::RigidBody;

/// Dejenere (sıfır) ölçek eksenini kırpan minimum yarı-genişlik — sıfır-kalınlık kutu
/// (broadphase/narrowphase'de dejenere manifold / NaN) oluşmasını önler.
pub const MIN_HE: f32 = 1e-4;

/// Ölçek + taban çarpanından kutu YARI-GENİŞLİĞİ türetir — boyut matematiğinin TEK kaynağı
/// (hem [`AutoBoxColliderSystem`] hem `Prefab` bunu çağırır, böylece iki yol asla ayrışmaz).
/// `.abs()` negatif ölçeğe karşı korur; `.max(MIN_HE)` dejenere ekseni kırpar. GPU'suz →
/// headless test edilebilir.
pub fn derived_box_half_extents(scale: Vec3, base: Vec3) -> Vec3 {
    (scale * base).abs().max(Vec3::splat(MIN_HE))
}

/// Opt-in İŞARET komponenti: "kutu collider'ımı `Transform.scale`'den boyutlandır."
///
/// Bir yer-tutucu kutu [`Collider`] ile birlikte ekle; [`AutoBoxColliderSystem`] ilk fizik
/// adımından ÖNCE çözer. `base` per-site çarpandır: mesh yarı-genişliği == ölçek olan
/// `create_cube` için [`AutoBoxCollider::new`] (base = `Vec3::ONE`); 0.5-faktörlü mesh ailesi
/// için [`AutoBoxCollider::scaled`]`(Vec3::splat(0.5))`.
///
/// NOT (hiyerarşi tuzağı): yerel `Transform.scale` okunur, birleşik dünya ölçeği DEĞİL — sıfır
/// olmayan ölçekli bir ebeveynin altındaki kutu yanlış boyutlanır (fiziğin geri kalanıyla
/// tutarlı: yerel Transform dünya kabul edilir). Ayrıca **çözüm bir kez** yapılır; spawn'dan
/// sonra `Transform.scale`'i değiştirirsen collider bayatlar (warm-start/blok-solver'ı her
/// frame perturbe etmemek için kasıtlı).
///
/// ⚠️ ZAMANLAMA TUZAĞI: bu marker, fizik adımından ÖNCE çalışan [`AutoBoxColliderSystem`] ile
/// `Added<T>` geçidinden çözülür. Windowed app döngüsünde `update` hook'u fizik `schedule.run`'
/// undan SONRA çalışır → **update hook'unda spawn'lanan** bir marker'ın `added_tick`'i bir
/// sonraki frame'in `change_ref_tick`'ine eşit olur ve strict `>` olan `Added` onu KAÇIRIR →
/// marker HİÇ çözülmez (collider yer-tutucuda kalır). Bu yüzden marker yolu YALNIZCA setup'ta
/// veya `physics_step`'ten önce çalışan bir SİSTEM içinde spawn'lanan varlıklar için güvenlidir.
/// Runtime (update-hook) spawn'ları için SENKRON yolu kullan:
/// [`Prefab::auto_box_collider`](crate::bundles::Prefab::auto_box_collider) veya açık
/// `Collider::box_collider(scale)`. Regresyon testi:
/// `marker_spawned_after_schedule_run_is_missed_by_added_gate`.
#[derive(Debug, Clone, Copy)]
pub struct AutoBoxCollider {
    /// Ölçekle çarpılan per-eksen taban çarpanı (genelde `Vec3::ONE`).
    pub base: Vec3,
}

impl AutoBoxCollider {
    /// `base = Vec3::ONE` — mesh yarı-genişliği == `Transform.scale` (ör. `create_cube`).
    pub fn new() -> Self {
        Self { base: Vec3::ONE }
    }

    /// Özel per-eksen taban çarpanı (ör. `Vec3::splat(0.5)` → yarı-genişlik = ölçek/2).
    pub fn scaled(base: Vec3) -> Self {
        Self { base }
    }
}

impl Default for AutoBoxCollider {
    fn default() -> Self {
        Self::new()
    }
}

gizmo_core::impl_component!(AutoBoxCollider);

/// Taze [`AutoBoxCollider`] işaretli varlıkların kutu collider'ını `Transform.scale`'den
/// boyutlandırır ve ataleti yeniden türetir; sonra işareti kaldırır. `Added<AutoBoxCollider>`
/// ile geçitlendiği için işaret başına TAM BİR KEZ çalışır (işaret kaldırılamasa bile). İşarete
/// dokunulmayan varlıklarla eşleşmez → determinizm-nötr. [`PhysicsPlugin`] bunu
/// `physics_step`'ten ÖNCE otomatik ekler.
///
/// [`PhysicsPlugin`]: crate::plugins::PhysicsPlugin
pub struct AutoBoxColliderSystem;

impl gizmo_core::system::System for AutoBoxColliderSystem {
    fn access_info(&self) -> gizmo_core::system::AccessInfo {
        let mut info = gizmo_core::system::AccessInfo::new();
        // Collider + RigidBody'ye mutable erişir ve Commands ile işareti kaldırır.
        info.is_exclusive = true;
        info
    }

    #[tracing::instrument(skip_all, level = "trace", name = "auto_box_collider")]
    fn run(&mut self, world: &World, dt: f32) {
        use gizmo_core::commands::Commands;
        use gizmo_core::query::{Added, Mut};
        use gizmo_core::system::SystemParam;

        // Commands YOKSA nazikçe küçül: yine de boyutlandır (Added geçidi doğruluğu korur),
        // yalnız işaret kaldırma atlanır → işaret öylece kalır (atıl).
        let mut commands = Commands::fetch(world, dt).ok();
        if commands.is_none() {
            tracing::trace!(
                "AutoBoxColliderSystem: Commands (CommandQueue) yok — işaretler boyutlanacak ama kaldırılamayacak"
            );
        }

        // Bu çalıştırmada gerçekten yapılan işi say → çıkışta tek AGGREGATE debug! (per-entity
        // log YOK; Added geçidi çoğu frame'de 0 işaret döndürür).
        let mut resolved = 0usize;
        let mut skipped_non_box = 0usize;
        let mut inertia_refreshed = 0usize;

        // ── PASS 1: collider'ı boyutlandır (+ işaret kaldırmayı kuyruğa al). ──
        // RigidBody GEREKTİRMEZ → trigger-only (RigidBody'siz) kutular da boyutlanır.
        // SAFETY: exclusive sistem; scheduler bu çalışırken disjoint mutable erişim garanti eder.
        if let Some(mut q) = unsafe {
            world
                .query_unchecked::<(&Transform, Mut<Collider>, &AutoBoxCollider, Added<AutoBoxCollider>)>()
        } {
            for (id, (t, mut col, cfg, _)) in q.iter_mut() {
                // Savunma: bir küre/kapsül collider'ı ASLA kutuya dönüştürme.
                if !matches!(col.shape, ColliderShape::Box(_)) {
                    skipped_non_box += 1;
                    tracing::warn!(
                        entity = id,
                        "AutoBoxCollider kutu-olmayan collider'a takılı — atlanıyor"
                    );
                    continue;
                }
                let he = derived_box_half_extents(t.scale, cfg.base);
                // YALNIZ .shape'e dokun → material/friction/restitution/layer/is_trigger korunur.
                col.shape = ColliderShape::Box(BoxShape { half_extents: he });
                resolved += 1;

                if let (Some(cmds), Some(e)) = (commands.as_mut(), world.entity(id)) {
                    cmds.entity(e).remove::<AutoBoxCollider>();
                }
            }
        }

        // ── PASS 2: ataleti tazele (yalnız RigidBody'si OLAN varlıklar). ──
        // Mevcut `update_inertia_from_collider`'ı YENİDEN KULLAN — Box kolu yarı→tam
        // ikilemeyi kendi içinde yapar, böylece FULL-vs-HALF ×2 tuzağı yapısal olarak imkânsız.
        // Statik/kinematik yazımı zararsızca yutar (inv_inertia=0); trigger-only doğal dışlanır.
        if let Some(mut q) = unsafe {
            world
                .query_unchecked::<(&Transform, Mut<RigidBody>, &Collider, &AutoBoxCollider, Added<AutoBoxCollider>)>()
        } {
            for (_id, (t, mut rb, col, cfg, _)) in q.iter_mut() {
                // Pass 1 ile SİMETRİ: kutu-olmayan collider'a takılı işarette Pass 1 şekli
                // KORUDU (atladı); burada da inertia'yı EZME — yoksa collider küre kalır ama
                // rb.local_inertia kutu-inertia olur (≫60× hata, tutarsız iki pass).
                if !matches!(col.shape, ColliderShape::Box(_)) {
                    continue;
                }
                let he = derived_box_half_extents(t.scale, cfg.base);
                rb.update_inertia_from_collider(&Collider::box_collider(he));
                inertia_refreshed += 1;
            }
        }

        if resolved > 0 || skipped_non_box > 0 {
            tracing::debug!(
                resolved,
                inertia_refreshed,
                skipped_non_box,
                "AutoBoxCollider: taze işaretler Transform.scale'den çözüldü"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_core::commands::CommandQueue;
    use gizmo_core::system::System;

    fn world_with_commands() -> World {
        let mut world = World::new();
        world.insert_resource(CommandQueue::default());
        world
    }

    // "physics_step" etiketli bir sonda-çalışan prob: çalıştığı anda gördüğü ilk kutu
    // collider'ının yarı-genişliğini bir kaynağa yazar → resolver ondan ÖNCE çözdüyse
    // ölçekli değeri yakalar.
    #[derive(Default)]
    struct ProbeCapture {
        he: Option<Vec3>,
    }

    struct ProbeSystem;
    impl System for ProbeSystem {
        fn access_info(&self) -> gizmo_core::system::AccessInfo {
            let mut i = gizmo_core::system::AccessInfo::new();
            i.is_exclusive = true;
            i
        }
        fn run(&mut self, world: &World, _dt: f32) {
            let mut captured = None;
            if let Some(q) = world.query::<&Collider>() {
                for (_id, col) in q.iter() {
                    if let ColliderShape::Box(b) = &col.shape {
                        captured = Some(b.half_extents);
                        break;
                    }
                }
            }
            if let Some(mut cap) = world.get_resource_mut::<ProbeCapture>() {
                cap.he = captured;
            }
        }
    }

    struct NoopSystem;
    impl System for NoopSystem {
        fn access_info(&self) -> gizmo_core::system::AccessInfo {
            gizmo_core::system::AccessInfo::new()
        }
        fn run(&mut self, _w: &World, _dt: f32) {}
    }

    /// TUZAK BELGESİ: bir marker fizik `schedule.run`'ından SONRA (ör. update hook'unda)
    /// spawn'lanırsa, `added_tick`'i bir sonraki frame'in `change_ref_tick`'ine EŞİT olur →
    /// strict `>` olan `Added` onu KAÇIRIR → marker HİÇ çözülmez. Bu yüzden update-hook'ta
    /// spawn'lanan yük-taşıyan statikler marker yoluyla DEĞİL, senkron yol (Prefab veya açık
    /// collider) ile boyutlanmalı. (yikim_ustasi bölüm-geçişi bu yüzden statikleri açık
    /// collider ile spawn'lar.)
    #[test]
    fn marker_spawned_after_schedule_run_is_missed_by_added_gate() {
        use gizmo_core::system::{Schedule, SystemConfig};

        let mut world = world_with_commands();
        let mut schedule = Schedule::new();
        schedule.add_di_system(
            SystemConfig::new(Box::new(AutoBoxColliderSystem))
                .label("auto_box_collider")
                .before("physics_step"),
        );
        schedule.add_di_system(SystemConfig::new(Box::new(NoopSystem)).label("physics_step"));

        // Frame 1: fizik loop'u (henüz marker yok).
        schedule.run(&mut world, 0.016);

        // "update hook" (fizikten SONRA): marker'lı statik spawn'la.
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::ZERO).with_scale(Vec3::new(12.0, 0.6, 10.0)));
        world.add_component(e, Collider::box_collider(Vec3::ONE));
        world.add_component(e, RigidBody::new_static());
        world.add_component(e, AutoBoxCollider::new());
        world.apply_commands();

        // Sonraki frame'lerin fizik loop'ları.
        schedule.run(&mut world, 0.016);
        schedule.run(&mut world, 0.016);

        // Marker çözülmedi → collider hâlâ yer-tutucu birim kutu (senkron yol şart).
        let col = world.borrow::<Collider>().get(e.id()).cloned().unwrap();
        match col.shape {
            ColliderShape::Box(b) => assert_eq!(
                b.half_extents,
                Vec3::ONE,
                "update-hook marker'ı Added ile çözülmez — tuzak belgelendi"
            ),
            _ => panic!("kutu olmalı"),
        }
    }

    /// Ordering-edge: resolver `.before("physics_step")` ile bağlanmalı → "physics_step"
    /// etiketli sistem çalıştığında collider ZATEN ölçekli kutu olmalı (ilk fizik adımı
    /// yer-tutucu birim kutuyla geçmez). Yanlış/eksik etiket sessizce düşse bu test kırılır.
    #[test]
    fn resolver_runs_before_physics_step_label() {
        use gizmo_core::system::{Schedule, SystemConfig};

        let mut world = world_with_commands();
        world.insert_resource(ProbeCapture::default());
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::ZERO).with_scale(Vec3::new(2.0, 3.0, 4.0)));
        world.add_component(e, Collider::box_collider(Vec3::ONE));
        world.add_component(e, RigidBody::new(1.0, true));
        world.add_component(e, AutoBoxCollider::new());

        let mut schedule = Schedule::new();
        schedule.add_di_system(
            SystemConfig::new(Box::new(AutoBoxColliderSystem))
                .label("auto_box_collider")
                .before("physics_step"),
        );
        schedule.add_di_system(SystemConfig::new(Box::new(ProbeSystem)).label("physics_step"));

        schedule.run(&mut world, 0.016); // schedule begin_change_frame'i kendi çağırır

        let cap = world.get_resource::<ProbeCapture>().unwrap();
        assert_eq!(
            cap.he,
            Some(Vec3::new(2.0, 3.0, 4.0)),
            "physics_step çalıştığında collider ölçekli olmalıydı (resolver .before ile bağlanmadı mı?)"
        );
    }

    /// Spawn bir kutu + Transform.scale + işaret; sistem collider'ı ölçeğe eşitlemeli, ataleti
    /// aynı ölçekli kutunun ataletiyle bire bir eşleştirmeli, işareti kaldırmalı.
    #[test]
    fn resolves_box_from_scale_and_derives_inertia() {
        let mut world = world_with_commands();
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::ZERO).with_scale(Vec3::new(2.0, 3.0, 4.0)));
        world.add_component(e, Collider::box_collider(Vec3::ONE)); // yer-tutucu
        world.add_component(e, RigidBody::new(10.0, true));
        world.add_component(e, AutoBoxCollider::new());

        world.begin_change_frame(0); // Added penceresini aç
        AutoBoxColliderSystem.run(&world, 0.016);
        world.apply_commands();

        // collider yarı-genişliği == ölçek
        let col = world.borrow::<Collider>().get(e.id()).cloned().unwrap();
        match col.shape {
            ColliderShape::Box(b) => assert_eq!(b.half_extents, Vec3::new(2.0, 3.0, 4.0)),
            _ => panic!("kutu olmalı"),
        }

        // atalet: referans gövdeyle bire bir (FULL-vs-HALF kilidi)
        let mut reference = RigidBody::new(10.0, true);
        reference.update_inertia_from_collider(&Collider::box_collider(Vec3::new(2.0, 3.0, 4.0)));
        let rb = world.borrow::<RigidBody>().get(e.id()).cloned().unwrap();
        assert_eq!(rb.local_inertia, reference.local_inertia);

        // işaret kaldırıldı
        assert!(world.borrow::<AutoBoxCollider>().get(e.id()).is_none());
    }

    /// base = 0.5 → yarı-genişlik = ölçek / 2 (0.5-faktörlü mesh ailesi).
    #[test]
    fn base_factor_halves_extents() {
        let mut world = world_with_commands();
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::ZERO).with_scale(Vec3::splat(4.0)));
        world.add_component(e, Collider::box_collider(Vec3::ONE));
        world.add_component(e, RigidBody::new(1.0, true));
        world.add_component(e, AutoBoxCollider::scaled(Vec3::splat(0.5)));

        world.begin_change_frame(0);
        AutoBoxColliderSystem.run(&world, 0.016);
        world.apply_commands();

        let col = world.borrow::<Collider>().get(e.id()).cloned().unwrap();
        match col.shape {
            ColliderShape::Box(b) => assert_eq!(b.half_extents, Vec3::splat(2.0)),
            _ => panic!("kutu olmalı"),
        }
    }

    /// Added geçidi: işaret KALDIRILMASA bile (Commands yok) sistem ikinci frame'de yeniden
    /// boyutlandırmamalı — idempotentlik işaretin varlığına değil, Added'a dayanır.
    #[test]
    fn runs_once_via_added_gate_without_commands() {
        let mut world = World::new(); // CommandQueue YOK → işaret kalır
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::ZERO).with_scale(Vec3::new(2.0, 2.0, 2.0)));
        world.add_component(e, Collider::box_collider(Vec3::ONE));
        world.add_component(e, RigidBody::new(1.0, true));
        world.add_component(e, AutoBoxCollider::new());

        world.begin_change_frame(0);
        AutoBoxColliderSystem.run(&world, 0.016);
        // Commands yok → işaret hâlâ orada
        assert!(world.borrow::<AutoBoxCollider>().get(e.id()).is_some(), "işaret kalmalı");

        // Frame 2: birileri collider'ı bozsun, sonra sistem YENİDEN çalışsın.
        // Added artık tetiklenmediği için collider dokunulmamış kalmalı.
        {
            let mut q = world.borrow_mut::<Collider>();
            let mut c = q.get_mut(e.id()).unwrap();
            c.shape = ColliderShape::Box(BoxShape { half_extents: Vec3::splat(9.0) });
        }
        let prev = world.tick;
        world.begin_change_frame(prev);
        AutoBoxColliderSystem.run(&world, 0.016);

        let col = world.borrow::<Collider>().get(e.id()).cloned().unwrap();
        match col.shape {
            // 9.0 korunmalı — sistem yeniden boyutlandırmadı (Added kapalı).
            ColliderShape::Box(b) => assert_eq!(b.half_extents, Vec3::splat(9.0)),
            _ => panic!("kutu olmalı"),
        }
    }

    /// Kutu-olmayan collider (küre) işaretli olsa bile DEĞİŞTİRİLMEMELİ.
    #[test]
    fn non_box_collider_is_skipped() {
        let mut world = world_with_commands();
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::ZERO).with_scale(Vec3::splat(3.0)));
        world.add_component(e, Collider::sphere(0.5));
        world.add_component(e, RigidBody::new(1.0, true));
        world.add_component(e, AutoBoxCollider::new());

        world.begin_change_frame(0);
        AutoBoxColliderSystem.run(&world, 0.016);
        world.apply_commands();

        let col = world.borrow::<Collider>().get(e.id()).cloned().unwrap();
        assert!(matches!(col.shape, ColliderShape::Sphere(_)), "küre korunmalı");
    }

    /// Dejenere (sıfır) ölçek ekseni MIN_HE'ye kırpılmalı — sıfır-kalınlık kutu yok.
    #[test]
    fn degenerate_scale_is_clamped() {
        let mut world = world_with_commands();
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::ZERO).with_scale(Vec3::new(0.0, 2.0, 3.0)));
        world.add_component(e, Collider::box_collider(Vec3::ONE));
        world.add_component(e, RigidBody::new(1.0, true));
        world.add_component(e, AutoBoxCollider::new());

        world.begin_change_frame(0);
        AutoBoxColliderSystem.run(&world, 0.016);
        world.apply_commands();

        let col = world.borrow::<Collider>().get(e.id()).cloned().unwrap();
        match col.shape {
            ColliderShape::Box(b) => {
                assert_eq!(b.half_extents.x, MIN_HE);
                assert_eq!(b.half_extents.y, 2.0);
            }
            _ => panic!("kutu olmalı"),
        }
    }

    /// Collider material (sürtünme/sekme) yeniden boyutlandırmada HAYATTA KALMALI.
    #[test]
    fn resize_preserves_material() {
        let mut world = world_with_commands();
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::ZERO).with_scale(Vec3::splat(2.0)));
        world.add_component(
            e,
            Collider::box_collider(Vec3::ONE)
                .with_friction(0.85)
                .with_restitution(0.3),
        );
        world.add_component(e, RigidBody::new(1.0, true));
        world.add_component(e, AutoBoxCollider::new());

        world.begin_change_frame(0);
        AutoBoxColliderSystem.run(&world, 0.016);
        world.apply_commands();

        let col = world.borrow::<Collider>().get(e.id()).cloned().unwrap();
        match col.shape {
            ColliderShape::Box(b) => assert_eq!(b.half_extents, Vec3::splat(2.0)),
            _ => panic!("kutu olmalı"),
        }
        assert_eq!(col.material.static_friction, 0.85);
        assert_eq!(col.material.restitution, 0.3);
    }

    /// Trigger-only (RigidBody'siz) varlık: collider boyutlanmalı, Pass 2 panik atmamalı.
    #[test]
    fn trigger_only_without_rigidbody_no_panic() {
        let mut world = world_with_commands();
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::ZERO).with_scale(Vec3::splat(5.0)));
        let mut trig = Collider::box_collider(Vec3::ONE);
        trig.is_trigger = true;
        world.add_component(e, trig);
        world.add_component(e, AutoBoxCollider::new());

        world.begin_change_frame(0);
        AutoBoxColliderSystem.run(&world, 0.016); // RigidBody yok → Pass 2 no-match, panik yok
        world.apply_commands();

        let col = world.borrow::<Collider>().get(e.id()).cloned().unwrap();
        match col.shape {
            ColliderShape::Box(b) => assert_eq!(b.half_extents, Vec3::splat(5.0)),
            _ => panic!("kutu olmalı"),
        }
        assert!(col.is_trigger, "trigger bayrağı korunmalı");
    }

    /// Statik gövde: collider boyutlanmalı, atalet yazımı zararsız (panik yok).
    #[test]
    fn static_body_resizes_without_panic() {
        let mut world = world_with_commands();
        let e = world.spawn();
        world.add_component(
            e,
            Transform::new(Vec3::ZERO).with_scale(Vec3::new(600.0, 1.0, 600.0)),
        );
        world.add_component(e, Collider::box_collider(Vec3::ONE));
        world.add_component(e, RigidBody::new_static());
        world.add_component(e, AutoBoxCollider::new());

        world.begin_change_frame(0);
        AutoBoxColliderSystem.run(&world, 0.016);
        world.apply_commands();

        let col = world.borrow::<Collider>().get(e.id()).cloned().unwrap();
        match col.shape {
            ColliderShape::Box(b) => assert_eq!(b.half_extents, Vec3::new(600.0, 1.0, 600.0)),
            _ => panic!("kutu olmalı"),
        }
    }

    /// Saf yardımcı: negatif ölçek mutlaklanır, base çarpılır, dejenere kırpılır.
    #[test]
    fn derived_helper_is_pure_and_guards() {
        assert_eq!(
            derived_box_half_extents(Vec3::new(2.0, 0.5, 2.0), Vec3::ONE),
            Vec3::new(2.0, 0.5, 2.0)
        );
        assert_eq!(
            derived_box_half_extents(Vec3::splat(4.0), Vec3::splat(0.5)),
            Vec3::splat(2.0)
        );
        // negatif ölçek → mutlak değer
        assert_eq!(
            derived_box_half_extents(Vec3::new(-3.0, 2.0, 1.0), Vec3::ONE),
            Vec3::new(3.0, 2.0, 1.0)
        );
        // sıfır → MIN_HE
        assert_eq!(derived_box_half_extents(Vec3::ZERO, Vec3::ONE), Vec3::splat(MIN_HE));
    }
}
