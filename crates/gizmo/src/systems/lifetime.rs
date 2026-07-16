//! Otomatik-despawn YAŞAM-DÖNGÜSÜ komponentleri + sistemi.
//!
//! "N saniye sonra sil" ve "kill-plane'in altına düşünce sil" gibi geçici-varlık
//! temizliği HEMEN her oyunda tekrarlanır — demolar bunu her frame elle `Vec<Entity>`
//! tutup, süre/konum kontrol edip `despawn` döngüsü yazarak yapar. Bir komponent ekle,
//! [`LifetimePlugin`]'i çalıştır, motor silsin (tıpkı [`Spin`](crate::systems::spin) gibi).
//!
//! ```ignore
//! world.add_component(spark, DespawnAfter::secs(2.0));   // 2 sn sonra yok
//! world.add_component(ball,  DespawnBelowY::new(-60.0)); // y < -60 olunca yok
//! app.add_plugin(LifetimePlugin);                        // otomatik temizlik
//! ```

use gizmo_core::world::World;
use gizmo_physics_core::Transform;

/// Entity'yi `remaining` saniye sonra otomatik despawn eder. Sistem her frame
/// `remaining`'i `dt` kadar azaltır; ≤ 0 olunca varlık silinir. (Mermi izi, kıvılcım,
/// konfeti, geçici ses/efekt kaynağı… için.)
#[derive(Debug, Clone, Copy)]
pub struct DespawnAfter {
    /// Kalan ömür (saniye). Runtime'da değiştirilebilir (ör. ömrü uzat).
    pub remaining: f32,
}

impl DespawnAfter {
    /// `secs` saniye sonra despawn olacak komponent.
    pub fn secs(secs: f32) -> Self {
        Self { remaining: secs }
    }
}

gizmo_core::impl_component!(DespawnAfter);

/// Entity'nin dünya-y konumu `y`'nin ALTINA inince otomatik despawn eder (kill-plane).
/// Uçuruma/boşluğa düşen gülleleri, saçılan enkazı elle izlemek yerine kullan.
#[derive(Debug, Clone, Copy)]
pub struct DespawnBelowY {
    /// Bu y-değerinin altına inen varlık silinir.
    pub y: f32,
}

impl DespawnBelowY {
    /// `y`'nin altına inince despawn olacak komponent.
    pub fn new(y: f32) -> Self {
        Self { y }
    }
}

gizmo_core::impl_component!(DespawnBelowY);

/// Süresi dolan ([`DespawnAfter`]) veya kill-plane'i geçen ([`DespawnBelowY`]) varlıkları
/// despawn eder. [`LifetimePlugin`] bunu schedule'a ekler; el ile `LifetimeSystem.run` da
/// çağrılabilir. Silme `Commands` ile ERTELENİR (schedule batch'ler arası flush eder).
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

        let mut commands = match Commands::fetch(world, dt) {
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
        if let Some(mut q) =
            unsafe { world.query_unchecked::<gizmo_core::query::Mut<DespawnAfter>>() }
        {
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

/// [`LifetimeSystem`]'i uygulamanın schedule'ına ekler → [`DespawnAfter`] /
/// [`DespawnBelowY`] komponentli varlıklar otomatik silinir.
pub struct LifetimePlugin;

impl<State: 'static> crate::app::Plugin<State> for LifetimePlugin {
    fn build(&self, app: &mut crate::app::App<State>) {
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
