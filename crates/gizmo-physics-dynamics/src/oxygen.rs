//! Basit oksijen/nefes sistemi — su-altı keşif oyunları (Subnautica-tarzı) için.
//!
//! Kafası batık entity'lerin oksijeni tükenir, yüzeye çıkınca dolar. Su hacimleri
//! [`PhysicsWorld::is_submerged`](gizmo_physics_rigid::world::PhysicsWorld) ile sorgulanır —
//! yani buoyancy, yüzme kontrolcüsü ve su-altı sisiyle AYNI `FluidZone`'ları kullanır.

use gizmo_core::component::IsDeleted;
use gizmo_core::query::{Mut, Without};
use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::Transform;

/// Bir entity'nin hava/oksijen deposu.
#[derive(Clone, Copy, Debug)]
pub struct Oxygen {
    /// Kalan hava (saniye).
    pub current: f32,
    /// Maksimum hava (saniye).
    pub max: f32,
    /// Batıkken saniyede tükenme miktarı.
    pub depletion_rate: f32,
    /// Yüzeyde saniyede dolma miktarı.
    pub refill_rate: f32,
    /// Ağız/burun yüksekliği (entity merkezinden Y ofseti); bu NOKTA suya batıksa oksijen tükenir.
    pub head_offset: f32,
}

impl gizmo_core::component::Component for Oxygen {}

impl Default for Oxygen {
    fn default() -> Self {
        Self {
            current: 45.0,
            max: 45.0,
            depletion_rate: 1.0,
            refill_rate: 6.0,
            head_offset: 0.6,
        }
    }
}

impl Oxygen {
    /// Kalan oran (0..1) — HUD barı için.
    #[inline]
    pub fn fraction(&self) -> f32 {
        if self.max > 0.0 {
            (self.current / self.max).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// Hava bitti mi (boğulma sınırı).
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.current <= 0.0
    }
}

/// `Oxygen` taşıyan her entity'nin havasını günceller: KAFA noktası (`position + head_offset`) bir
/// su hacminde (FluidZone) ise tüketir, değilse doldurur. `PhysicsWorld` kaynağı yoksa (sahnede su
/// yok) her şey dolar — sistemin su içermeyen sahnelerde no-op olması garanti.
#[tracing::instrument(skip_all, name = "oxygen_system")]
pub fn oxygen_system(world: &World, dt: f32) {
    if dt <= 0.0 {
        return;
    }
    let phys = world.get_resource::<gizmo_physics_rigid::world::PhysicsWorld>();

    // SAFETY: exclusive `fn(&World, f32)` sistemi — scheduler tek başına çalıştırır; `Oxygen` ve
    // `Transform` ayrı bileşen tipleri, alias yok.
    let query = unsafe { world.query_unchecked::<(Mut<Oxygen>, &Transform, Without<IsDeleted>)>() };
    if let Some(mut query) = query {
        for (_id, (mut oxy, transform, _)) in query.iter_mut() {
            let head = transform.position + Vec3::new(0.0, oxy.head_offset, 0.0);
            let submerged = phys.as_ref().is_some_and(|pw| pw.is_submerged(head));
            if submerged {
                let before = oxy.current;
                oxy.current = (oxy.current - oxy.depletion_rate * dt).max(0.0);
                // Havanın bittiği ANda (boğulma sınırı geçişi) bir kez logla — per-frame değil.
                if before > 0.0 && oxy.current <= 0.0 {
                    tracing::debug!(
                        entity_id = _id,
                        depletion_rate = oxy.depletion_rate,
                        head_y = head.y,
                        "[Oxygen] entity ran out of air (drowning threshold reached)"
                    );
                }
            } else {
                oxy.current = (oxy.current + oxy.refill_rate * dt).min(oxy.max);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_physics_rigid::world::{FluidZone, PhysicsWorld, ZoneShape};

    fn world_with_water() -> World {
        let mut world = World::new();
        let mut pw = PhysicsWorld::new();
        pw.fluid_zones.push(FluidZone {
            shape: ZoneShape::Box {
                min: Vec3::new(-50.0, -30.0, -50.0),
                max: Vec3::new(50.0, 0.0, 50.0), // yüzey y=0
            },
            ..Default::default()
        });
        world.insert_resource(pw);
        world
    }

    /// Batıkken oksijen tükenir, yüzeye çıkınca dolar.
    #[test]
    fn oxygen_depletes_underwater_and_refills_at_surface() {
        let mut world = world_with_water();
        let diver = world.spawn();
        world.add_component(diver, Transform::new(Vec3::new(0.0, -5.0, 0.0))); // kafa -4.4 → suda
        world.add_component(
            diver,
            Oxygen { current: 20.0, max: 20.0, depletion_rate: 2.0, refill_rate: 5.0, head_offset: 0.6 },
        );
        let dt = 1.0 / 60.0;
        for _ in 0..60 {
            oxygen_system(&world, dt); // 1 sn batık
        }
        let o = world.query::<&Oxygen>().unwrap().get(diver.id()).unwrap().current;
        assert!(o < 20.0 - 1.5, "batıkken oksijen tükenmeli (~2/sn), bulundu {o}");

        // Yüzeye taşı (kafa +2.6 → havada) → dolmalı.
        world.borrow_mut::<Transform>().get_mut(diver.id()).unwrap().position =
            Vec3::new(0.0, 2.0, 0.0);
        for _ in 0..60 {
            oxygen_system(&world, dt);
        }
        let o2 = world.query::<&Oxygen>().unwrap().get(diver.id()).unwrap().current;
        assert!(o2 > o, "yüzeyde oksijen dolmalı, {o} → {o2}");
    }

    /// `fraction()` is a clamped ratio for the HUD bar: 1.0 when full, 0.5 at half, and
    /// clamped into [0,1] when `current` is over-full or negative. Crucially it guards
    /// `max <= 0` (division by zero) by returning 0.0 instead of NaN/Inf.
    #[test]
    fn fraction_clamps_and_guards_zero_max() {
        let mk = |current, max| Oxygen { current, max, ..Default::default() };
        assert!((mk(45.0, 45.0).fraction() - 1.0).abs() < 1e-6, "full → 1.0");
        assert!((mk(10.0, 20.0).fraction() - 0.5).abs() < 1e-6, "half → 0.5");
        assert_eq!(mk(30.0, 20.0).fraction(), 1.0, "over-full clamps to 1.0");
        assert_eq!(mk(-5.0, 20.0).fraction(), 0.0, "negative current clamps to 0.0");
        // Division-by-zero guard.
        let f = mk(5.0, 0.0).fraction();
        assert_eq!(f, 0.0, "max <= 0 → 0.0, not NaN");
        assert!(f.is_finite());
        assert_eq!(mk(5.0, -1.0).fraction(), 0.0, "negative max → 0.0");
    }

    /// `is_empty()` (the drowning threshold) is true at exactly zero and below, false
    /// just above zero.
    #[test]
    fn is_empty_boundary_at_zero() {
        let mk = |current| Oxygen { current, max: 10.0, ..Default::default() };
        assert!(mk(0.0).is_empty(), "exactly empty counts as empty");
        assert!(mk(-0.5).is_empty(), "negative counts as empty");
        assert!(!mk(0.001).is_empty(), "a sliver of air is not empty");
    }

    /// Refill invariant: `current` never exceeds `max` no matter how long/fast it
    /// refills (the `.min(max)` cap holds under a huge refill rate + many steps).
    #[test]
    fn refill_never_exceeds_max() {
        let mut world = World::new(); // no water → always refilling
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::ZERO));
        world.add_component(
            e,
            Oxygen { current: 1.0, max: 10.0, depletion_rate: 1.0, refill_rate: 100.0, head_offset: 0.6 },
        );
        for _ in 0..600 {
            oxygen_system(&world, 1.0 / 60.0); // 100/s × 10s ≫ max, must still cap
        }
        let o = world.query::<&Oxygen>().unwrap().get(e.id()).unwrap().current;
        assert!((o - 10.0).abs() < 1e-4, "refill must cap at max, got {o}");
    }

    /// Depletion invariant: submerged `current` floors at 0 and never goes negative,
    /// even with a huge depletion rate over many steps (the `.max(0.0)` floor holds).
    #[test]
    fn depletion_floors_at_zero() {
        let mut world = world_with_water();
        let diver = world.spawn();
        world.add_component(diver, Transform::new(Vec3::new(0.0, -10.0, 0.0))); // head -9.4 → suda
        world.add_component(
            diver,
            Oxygen { current: 2.0, max: 10.0, depletion_rate: 100.0, refill_rate: 1.0, head_offset: 0.6 },
        );
        for _ in 0..600 {
            oxygen_system(&world, 1.0 / 60.0);
        }
        let o = world.query::<&Oxygen>().unwrap().get(diver.id()).unwrap().current;
        assert_eq!(o, 0.0, "depletion must floor at exactly 0, got {o}");
        assert!(o >= 0.0, "oxygen must never go negative");
    }

    /// `dt <= 0` is an early-return no-op: a zero or negative timestep must not deplete
    /// or refill (guards paused frames / reversed time from perturbing the store).
    #[test]
    fn non_positive_dt_is_a_noop() {
        let mut world = world_with_water();
        let diver = world.spawn();
        world.add_component(diver, Transform::new(Vec3::new(0.0, -5.0, 0.0))); // submerged
        world.add_component(diver, Oxygen { current: 5.0, ..Default::default() });
        oxygen_system(&world, 0.0);
        oxygen_system(&world, -1.0);
        let o = world.query::<&Oxygen>().unwrap().get(diver.id()).unwrap().current;
        assert_eq!(o, 5.0, "dt <= 0 must leave oxygen untouched, got {o}");
    }

    /// Sahnede su (PhysicsWorld) yoksa boğulma olmaz — oksijen dolar (no-op güvenliği).
    #[test]
    fn oxygen_refills_without_physics_world() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::ZERO));
        world.add_component(e, Oxygen { current: 5.0, ..Default::default() });
        for _ in 0..60 {
            oxygen_system(&world, 1.0 / 60.0);
        }
        let o = world.query::<&Oxygen>().unwrap().get(e.id()).unwrap().current;
        assert!(o > 5.0, "susuz dünyada oksijen dolmalı, bulundu {o}");
    }
}
