use gizmo_math::Vec3;
use serde::{Deserialize, Serialize};

/// How an [`Explosion`]'s strength decays with distance from its centre.
///
/// Every curve is a function of `dist / radius` alone and is `0` at and beyond the radius,
/// so the choice never extends an explosion's reach — it only redistributes strength inside
/// it. All curves reach full strength (`1.0`) exactly at the centre, and the same value
/// scales both the impulse and the damage of the blast.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub enum ExplosionFalloff {
    /// No decay: full strength anywhere inside the radius, then a hard step to zero at it.
    /// The cheapest to reason about and the most likely to look wrong — two bodies a
    /// millimetre either side of the rim are hit with everything and nothing.
    None, // Sabit kuvvet
    /// `1 - dist/radius`: strength drops evenly to zero at the rim, so the edge of the
    /// blast is continuous rather than a step. This is what [`Default`] yields.
    #[default]
    Linear, // 1 - (dist / radius)
    /// `(1 - dist/radius)²`: same endpoints as [`Linear`](Self::Linear) but below it
    /// everywhere in between, which concentrates the blast near the centre and softens the
    /// rim further. Note this is the square of the linear ramp, *not* an inverse-square law.
    Quadratic, // (1 - dist/radius)^2
}

/// A one-shot radial blast: an outward impulse on every dynamic body in range, plus damage
/// to every [`Breakable`](super::Breakable) in range.
///
/// Put it on an entity that also has a `Transform`; the blast originates at that transform's
/// position plus [`offset`](Self::offset). It is a request, not a record of an event — the
/// built-in `physics_explosion_system` applies an active explosion exactly once and then
/// despawns the entity, so game code that wants to observe the blast has to be scheduled
/// ahead of it.
///
/// The blast is geometric and unobstructed: distances are measured to other bodies'
/// transform positions, walls do not shadow it, and nothing about it is integrated over
/// time, so its effect does not depend on the frame rate or substep count.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Explosion {
    /// Reach of the blast in metres, measured from the blast origin to each body's
    /// `Transform` position — body origins, not collider surfaces and not centres of mass,
    /// so a large object is either wholly in range or wholly out of it. Bodies at or beyond
    /// this distance are untouched, as are bodies closer than about 0.03 m (the squared
    /// distance must exceed `0.001`), where there is no well-defined direction to push in.
    pub force_radius: f32,
    /// Blast strength at the centre, applied as an **impulse**, not as a force held over
    /// time: an affected body's velocity changes by `force · falloff / mass` in a single
    /// step with no `dt` scaling, so the units are effectively N·s (kg·m/s) despite the
    /// name. The push is central — it adds linear velocity only and imparts no spin.
    pub force: f32,
    /// Health subtracted from a [`Breakable`](super::Breakable) at the blast's centre,
    /// scaled down by the same falloff curve as [`force`](Self::force). Bodies without a
    /// `Breakable` take no damage at all; this is a destruction input, not a general
    /// gameplay damage channel. Damage is only dealt where the scaled impulse also clears
    /// that breakable's `threshold`, so a blast too weak to move an object cannot chip it
    /// either.
    pub damage: f32,
    /// Intended reach of the damage effect in metres, kept separate from
    /// [`force_radius`](Self::force_radius) so a blast could shove further than it hurts.
    ///
    /// **Not currently honoured.** No system in this workspace reads this field; the
    /// built-in explosion pass tests breakables against `force_radius`. Setting it changes
    /// nothing in the simulation today.
    pub damage_radius: f32,
    /// Which [`ExplosionFalloff`] curve weights [`force`](Self::force) and
    /// [`damage`](Self::damage) at a given distance — one curve for both.
    pub falloff: ExplosionFalloff,
    /// Blast origin relative to the entity's `Transform` position, in metres along **world**
    /// axes: it is added unrotated, so rotating the entity does not swing the offset around
    /// with it. `Vec3::ZERO` fires the blast at the transform origin.
    pub offset: Vec3,
    /// Whether the explosion is armed. Only an armed explosion is applied — and being
    /// applied is also what despawns the entity, so leaving this `false` parks a reusable,
    /// inert explosion on an entity until something arms it. Defaults to `true`, so a
    /// freshly spawned `Explosion` detonates on the next physics run.
    pub is_active: bool,
}

impl Default for Explosion {
    fn default() -> Self {
        Self {
            force_radius: 5.0,
            force: 1000.0,
            damage: 100.0,
            damage_radius: 5.0,
            falloff: ExplosionFalloff::Linear,
            offset: Vec3::ZERO,
            is_active: true,
        }
    }
}

gizmo_core::impl_component!(Explosion);
