//! Auto-despawn LIFECYCLE components + system.
//!
//! Temporary-entity cleanup like "delete after N seconds" and "delete when it falls below the
//! kill-plane" repeats in ALMOST every game — the demos do it by hand every frame by keeping a
//! `Vec<Entity>`, checking time/position and writing a `despawn` loop. Add one component, run
//! [`LifetimePlugin`], let the engine delete (just like [`Spin`](crate::systems::spin)).
//!
//! ```
//! use gizmo::prelude::*;
//! use gizmo::core::system::System;
//! use gizmo::systems::lifetime::LifetimeSystem;
//!
//! let mut world = World::new();
//!
//! let spark = world.spawn();
//! world.add_component(spark, Transform::new(Vec3::ZERO));
//! world.add_component(spark, DespawnAfter::secs(2.0));
//!
//! let ball = world.spawn();
//! world.add_component(ball, Transform::new(Vec3::new(0.0, -100.0, 0.0)));
//! world.add_component(ball, DespawnBelowY::new(-60.0));
//!
//! // In an application `app.add_plugin(LifetimePlugin)` runs this every frame.
//! let mut sys = LifetimeSystem;
//! sys.run(&world, 1.0);
//! world.apply_commands();
//! assert!(world.is_alive(spark), "after 1 s the spark is still alive");
//! assert!(!world.is_alive(ball), "the ball below the threshold must be despawned at once");
//!
//! sys.run(&world, 1.5); // toplam 2.5 sn > 2.0
//! world.apply_commands();
//! assert!(!world.is_alive(spark), "the expired spark must be despawned");
//! ```

use gizmo_core::world::World;
use gizmo_physics_core::Transform;

/// Automatically despawns the entity after `remaining` seconds. Every frame the system
/// decreases `remaining` by `dt`; when it is ≤ 0 the entity is deleted. (For bullet trails,
/// sparks, confetti, a temporary sound/effect source…)
///
/// # Inert without [`LifetimePlugin`]
///
/// The system that reads this component ships with [`LifetimePlugin`], and that plugin is **not
/// on by default**. Without it the component attaches, nothing reads it, the entity never dies,
/// and **nothing warns** — entities simply accumulate.
///
/// Measured 2026-08-23 while building `demo/src/bin/delayed_commands.rs`: the same scene run
/// with and without the plugin grew 5 → 10 → 15 live entities in one case and drained correctly
/// in the other. It was caught only because that demo ran a hand-built delay queue beside the
/// engine's and could see the two diverge.
#[derive(Debug, Clone, Copy)]
pub struct DespawnAfter {
    /// Remaining lifetime (seconds). Can be changed at runtime (e.g. extend the lifetime).
    pub remaining: f32,
}

impl DespawnAfter {
    /// Component that will despawn after `secs` seconds.
    pub fn secs(secs: f32) -> Self {
        Self { remaining: secs }
    }
}

gizmo_core::impl_component!(DespawnAfter);

/// Automatically despawns once the entity's world-y position drops BELOW `y` (kill-plane).
/// Use it instead of hand-tracking cannonballs falling into a chasm/the void, or scattered
/// debris.
#[derive(Debug, Clone, Copy)]
pub struct DespawnBelowY {
    /// An entity that drops below this y value is deleted.
    pub y: f32,
}

impl DespawnBelowY {
    /// Component that will despawn once it drops below `y`.
    pub fn new(y: f32) -> Self {
        Self { y }
    }
}

gizmo_core::impl_component!(DespawnBelowY);

/// Despawns entities whose time has run out ([`DespawnAfter`]) or that have crossed the
/// kill-plane ([`DespawnBelowY`]). [`LifetimePlugin`] adds this to the schedule;
/// `LifetimeSystem.run` can also be called by hand. Deletion is DEFERRED via `Commands`
/// (the schedule flushes between batches).
pub struct LifetimeSystem;

impl gizmo_core::system::System for LifetimeSystem {
    fn access_info(&self) -> gizmo_core::system::AccessInfo {
        let mut info = gizmo_core::system::AccessInfo::new();
        // DespawnAfter'a mutable erişir + Commands ile (ertelenmiş) despawn eder.
        info.is_exclusive = true;
        info
    }

    #[tracing::instrument(skip_all, level = "trace", name = "lifetime")]
    fn run(&mut self, world: &World, dt: f32) {
        use gizmo_core::commands::Commands;
        use gizmo_core::system::SystemParam;

        let mut commands = match Commands::fetch_stateless(world, dt) {
            Ok(c) => c,
            // Sessiz `Err(_) => return` yutması yerine: CommandQueue yoksa hiçbir varlık
            // despawn edilemez (yaşam-döngüsü komponentleri atıl kalır). Kalıcı, per-frame
            // bir koşul olduğu için trace! (gürültü yapmaz; kurulum hatasında görünür).
            Err(_) => {
                tracing::trace!(
                    "LifetimeSystem: Commands (CommandQueue) yok — despawn atlanıyor, ömür komponentleri atıl"
                );
                return;
            }
        };

        // Silinenleri say → çıkışta tek AGGREGATE debug! (per-entity despawn logu YOK).
        let mut despawned_after = 0usize;
        let mut despawned_below_y = 0usize;

        // ── DespawnAfter: sayacı azalt, süresi dolanları sil. ──
        // SAFETY: exclusive sistem; scheduler bu çalışırken disjoint mutable erişim garanti eder.
        // (Bound out of the `if let` so the comment sits directly above the `unsafe`, which is
        // where `clippy::undocumented_unsafe_blocks` looks — the lint is a ratchet in this crate.)
        let despawn_after =
            unsafe { world.query_unchecked::<gizmo_core::query::Mut<DespawnAfter>>() };
        if let Some(mut q) = despawn_after {
            for (id, mut d) in q.iter_mut() {
                d.remaining -= dt;
                if d.remaining <= 0.0 {
                    if let Some(e) = world.entity(id) {
                        commands.entity(e).despawn();
                        despawned_after += 1;
                    }
                }
            }
        }

        // ── DespawnBelowY: konumu eşiğin altındaki varlıkları sil. ──
        // SAFETY: exclusive system; this view is read-only and aliases nothing mutably.
        if let Some(q) = unsafe { world.query_unchecked::<(&DespawnBelowY, &Transform)>() } {
            for (id, (below, t)) in q.iter() {
                if t.position.y < below.y {
                    if let Some(e) = world.entity(id) {
                        commands.entity(e).despawn();
                        despawned_below_y += 1;
                    }
                }
            }
        }

        if despawned_after > 0 || despawned_below_y > 0 {
            tracing::debug!(
                despawned_after,
                despawned_below_y,
                "LifetimeSystem: geçici varlıklar despawn için kuyruğa alındı"
            );
        }
    }
}

/// Adds [`LifetimeSystem`] to the application's schedule → entities with a [`DespawnAfter`] /
/// [`DespawnBelowY`] component are deleted automatically.
pub struct LifetimePlugin;

impl crate::app::Plugin for LifetimePlugin {
    fn build(&self, app: &mut dyn crate::app::AppLike) {
        let app = app.parts_mut();
        app.schedule.add_di_system(
            gizmo_core::system::SystemConfig::new(Box::new(LifetimeSystem)).label("lifetime"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_core::commands::CommandQueue;
    use gizmo_core::system::System;
    use gizmo_math::Vec3;

    fn world_with_commands() -> World {
        let mut world = World::new();
        world.insert_resource(CommandQueue::default());
        world
    }

    #[test]
    fn despawn_after_removes_entity_when_timer_elapses() {
        let mut world = world_with_commands();
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::ZERO));
        world.add_component(e, DespawnAfter::secs(0.1));

        let mut sys = LifetimeSystem;

        // 0.05 s: henüz canlı.
        sys.run(&world, 0.05);
        world.apply_commands();
        assert!(world.is_alive(e), "0.05s'de hâlâ canlı olmalı");

        // +0.1 s → toplam 0.15 > 0.1: silinmeli.
        sys.run(&world, 0.1);
        world.apply_commands();
        assert!(!world.is_alive(e), "süre dolunca despawn edilmeli");
    }

    #[test]
    fn despawn_below_y_removes_fallen_entity() {
        let mut world = world_with_commands();

        let above = world.spawn();
        world.add_component(above, Transform::new(Vec3::new(0.0, 5.0, 0.0)));
        world.add_component(above, DespawnBelowY::new(-60.0));

        let fallen = world.spawn();
        world.add_component(fallen, Transform::new(Vec3::new(0.0, -100.0, 0.0)));
        world.add_component(fallen, DespawnBelowY::new(-60.0));

        let mut sys = LifetimeSystem;
        sys.run(&world, 0.016);
        world.apply_commands();

        assert!(world.is_alive(above), "eşiğin üstündeki korunmalı");
        assert!(!world.is_alive(fallen), "eşiğin altındaki silinmeli");
    }
}
