//! Automatic BOX-COLLIDER — derive the size from `Transform.scale` (do NOT write the size TWICE).
//!
//! When spawning a box the developer has until now had to write the size twice: once as the
//! visual scale (`Transform::with_scale(half)`), once as the matching collider
//! (`Collider::box_collider(half)`). If the two diverge, physics and the visual silently come
//! apart from each other. This is the perfect example of "forcing the developer into low-level
//! repetition" that the user pointed out.
//!
//! Solution — an opt-in marker that follows the [`Spin`](crate::systems::spin) /
//! [`LifetimeSystem`](crate::systems::lifetime) idiom: add an [`AutoBoxCollider`] component (+ a
//! placeholder box collider) and let [`PhysicsPlugin`](crate::plugins::PhysicsPlugin) run the
//! system automatically. [`AutoBoxColliderSystem`] resolves every fresh marker once, BEFORE the
//! first physics step: `Transform.scale · base` → collider half-extents, inertia is re-derived,
//! then the marker is removed. The size is written only ONCE (as Transform.scale).
//!
//! There is also a synchronous short-cut for `Prefab`
//! ([`Prefab::auto_box_collider`](crate::bundles::Prefab::auto_box_collider)): since
//! `Prefab::spawn` already holds the transform in hand, it resolves the collider at spawn time
//! (no one-frame delay). Both paths use the same pure [`derived_box_half_extents`] helper.
//!
//! ```
//! use gizmo::prelude::*;
//! # use gizmo::core::system::System;
//! # use gizmo::systems::auto_collider::AutoBoxColliderSystem;
//! # use gizmo_physics_core::ColliderShape;
//! # let mut world = World::new();
//! // The raw (Prefab-less) path — works through any spawn channel:
//! let plank = world.spawn();
//! world.add_component(
//!     plank,
//!     Transform::new(Vec3::ZERO).with_scale(Vec3::new(4.0, 0.4, 1.0)), // the size, written ONCE
//! );
//! world.add_component(plank, Collider::box_collider(Vec3::ONE)); // placeholder
//! world.add_component(plank, AutoBoxCollider::new()); // resolved before the first physics step
//!
//! AutoBoxColliderSystem.run(&world, 1.0 / 60.0);
//! world.apply_commands();
//!
//! let colliders = world.borrow::<Collider>();
//! let shape = &colliders.get(plank.id()).unwrap().shape;
//! assert!(matches!(shape, ColliderShape::Box(b) if b.half_extents == Vec3::new(4.0, 0.4, 1.0)));
//! // …and the marker is gone, so the resolution happens exactly once.
//! assert!(world.borrow::<AutoBoxCollider>().get(plank.id()).is_none());
//! ```

use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::{BoxShape, Collider, ColliderShape, Transform};
use gizmo_physics_rigid::components::RigidBody;

/// Minimum half-extent that clamps a degenerate (zero) scale axis — prevents a zero-thickness
/// box (a degenerate manifold / NaN in broadphase/narrowphase) from forming.
pub const MIN_HE: f32 = 1e-4;

/// Derives the box HALF-EXTENTS from scale + base factor — the SINGLE source of the size math
/// (both [`AutoBoxColliderSystem`] and `Prefab` call this, so the two paths never diverge).
/// `.abs()` guards against negative scale; `.max(MIN_HE)` clamps a degenerate axis. No GPU →
/// headless testable.
pub fn derived_box_half_extents(scale: Vec3, base: Vec3) -> Vec3 {
    (scale * base).abs().max(Vec3::splat(MIN_HE))
}

/// Opt-in MARKER component: "size my box collider from `Transform.scale`."
///
/// Add it together with a placeholder box [`Collider`]; [`AutoBoxColliderSystem`] resolves it
/// BEFORE the first physics step. `base` is the per-site factor: [`AutoBoxCollider::new`]
/// (base = `Vec3::ONE`) for `create_cube`, whose mesh half-extent == scale;
/// [`AutoBoxCollider::scaled`]`(Vec3::splat(0.5))` for the 0.5-factor mesh family.
///
/// NOTE (hierarchy trap): the local `Transform.scale` is read, NOT the composed world scale — a
/// box under a parent with a non-zero scale is sized incorrectly (consistent with the rest of
/// the physics: the local Transform is taken to be the world one). Also, **resolution happens
/// once**; if you change `Transform.scale` after spawn the collider goes stale (deliberate, in
/// order not to perturb the warm-start/block-solver every frame).
///
/// NOTE(translation): the original says "non-zero scale" here; the intended sense is most likely
/// "non-unit scale" (a scale other than 1). Translated literally rather than corrected.
///
/// ⚠️ TIMING TRAP: this marker is resolved through the `Added<T>` gate by
/// [`AutoBoxColliderSystem`], which runs BEFORE the physics step. In the windowed app loop the
/// `update` hook runs AFTER the physics `schedule.run` → the `added_tick` of a marker
/// **spawned in the update hook** ends up equal to the next frame's `change_ref_tick`, and
/// `Added`, which is a strict `>`, MISSES it → the marker is NEVER resolved (the collider stays
/// at the placeholder). Therefore the marker path is safe ONLY for entities spawned in setup or
/// inside a SYSTEM that runs before `physics_step`. For runtime (update-hook) spawns use the
/// SYNCHRONOUS path: [`Prefab::auto_box_collider`](crate::bundles::Prefab::auto_box_collider) or
/// an explicit `Collider::box_collider(scale)`. Regression test:
/// `marker_spawned_after_schedule_run_is_missed_by_added_gate`.
#[derive(Debug, Clone, Copy)]
pub struct AutoBoxCollider {
    /// Per-axis base factor multiplied by the scale (usually `Vec3::ONE`).
    pub base: Vec3,
}

impl AutoBoxCollider {
    /// `base = Vec3::ONE` — mesh half-extent == `Transform.scale` (e.g. `create_cube`).
    pub fn new() -> Self {
        Self { base: Vec3::ONE }
    }

    /// Custom per-axis base factor (e.g. `Vec3::splat(0.5)` → half-extent = scale/2).
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

/// Sizes the box collider of entities carrying a fresh [`AutoBoxCollider`] marker from
/// `Transform.scale` and re-derives the inertia; then removes the marker. Because it is gated
/// with `Added<AutoBoxCollider>` it runs EXACTLY ONCE per marker (even if the marker cannot be
/// removed). It does not match entities whose marker is untouched → determinism-neutral.
/// [`PhysicsPlugin`] adds it automatically BEFORE `physics_step`.
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
        let mut commands = Commands::fetch_stateless(world, dt).ok();
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
        // SAFETY: scheduled system; RigidBody is the only mutable view and nothing else in
        // this pass holds one, so the borrow `query_unchecked` skips cannot be violated here.
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

    /// TRAP DOCUMENT: if a marker is spawned AFTER the physics `schedule.run` (e.g. in the update
    /// hook), its `added_tick` becomes EQUAL to the next frame's `change_ref_tick` →
    /// `Added`, which is a strict `>`, MISSES it → the marker is NEVER resolved. Therefore
    /// load-bearing statics spawned in the update hook must be sized NOT via the marker path but
    /// via the synchronous path (Prefab or an explicit collider). (This is why the yikim_ustasi
    /// level transition spawns its statics with an explicit collider.)
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

    /// Ordering-edge: the resolver must be wired with `.before("physics_step")` → by the time the
    /// system labeled "physics_step" runs the collider must ALREADY be a scaled box (the first
    /// physics step never runs with the placeholder unit box). If a wrong or missing label
    /// silently dropped the ordering, this test would break.
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

    /// Spawn a box + Transform.scale + marker; the system must match the collider to the scale,
    /// match the inertia one-to-one with the inertia of the same scaled box, and remove the marker.
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

    /// base = 0.5 → half-extent = scale / 2 (the 0.5-factor mesh family).
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

    /// Added gate: even if the marker is NOT REMOVED (no Commands) the system must not re-size on
    /// the second frame — idempotency rests on Added, not on the presence of the marker.
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

    /// A non-box collider (sphere) MUST NOT BE CHANGED even if it carries the marker.
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

    /// A degenerate (zero) scale axis must be clamped to MIN_HE — no zero-thickness box.
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

    /// The collider material (friction/bounce) MUST SURVIVE the re-size.
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

    /// Trigger-only (RigidBody-less) entity: the collider must be sized, Pass 2 must not panic.
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

    /// Static body: the collider must be sized, the inertia write is harmless (no panic).
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

    /// Pure helper: negative scale is made absolute, base is multiplied in, degenerate is clamped.
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
