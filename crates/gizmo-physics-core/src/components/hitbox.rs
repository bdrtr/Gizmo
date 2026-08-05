use gizmo_math::Vec3;

use serde::{Deserialize, Serialize};

/// An oriented box that *deals* damage while it is active — the attacking half of a
/// hitbox/hurtbox pair.
///
/// This is gameplay data, not collision geometry. A `Hitbox` is not a
/// [`Collider`](crate::components::Collider) and takes no part in broadphase, narrowphase or
/// the solver, so it generates no contacts and stops nothing from moving. Deciding when one
/// overlaps a [`Hurtbox`], and what that costs, is left to game code.
///
/// The box is axis-aligned in the owning entity's local frame and inherits that entity's
/// rotation, making it an oriented box in the world. Lengths are metres.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Hitbox {
    /// Centre of the box relative to the entity's `Transform`, expressed in the entity's
    /// local frame and in metres — it is placed at `position + rotation * offset`, so it turns
    /// with the fighter and holds still relative to it.
    ///
    /// `Vec3::ZERO` (the default) pins the box to the transform origin, which for a model
    /// authored feet-at-origin sits on the floor rather than at chest height.
    pub offset: Vec3,
    /// Half-sizes along the local X/Y/Z axes in metres, so the box spans twice this on each
    /// axis; the default `0.2` triple is a 0.4 m cube.
    ///
    /// Expected non-negative, but nothing checks: a zero component flattens the box to a plane
    /// or a point, and negatives are not folded to their absolute value — the volume is simply
    /// degenerate.
    pub half_extents: Vec3,
    /// Health points a clean hit with this box takes off, on the same scale as
    /// [`FighterController::health`](crate::components::fighter::FighterController::health) —
    /// `10.0` by default, against a default 100-point bar.
    ///
    /// Nothing subtracts it, because no engine system resolves these hits. When the box
    /// belongs to a move it duplicates
    /// [`FrameData::damage`](crate::components::fighter::FrameData::damage), and keeping the
    /// two in step is the game's problem.
    pub damage: f32,
    /// Whether the box counts this frame; `true` from both `default()` and [`Hitbox::new`], so
    /// a freshly built hitbox is live immediately.
    ///
    /// This is the flag a move's active window is meant to drive — clearing it through startup
    /// and recovery is what keeps an attack from connecting outside its window. Nothing in
    /// this crate reads it.
    pub active: bool,
}

impl Default for Hitbox {
    fn default() -> Self {
        Self {
            offset: Vec3::ZERO,
            half_extents: Vec3::new(0.2, 0.2, 0.2),
            damage: 10.0,
            active: true,
        }
    }
}

impl Hitbox {
    /// Builds an already-active hitbox centred on the entity's transform origin
    /// (`offset` = `Vec3::ZERO`).
    ///
    /// Both arguments are stored verbatim: nothing rejects a zero or negative extent, nor
    /// negative damage. Assign `offset` afterwards to move the box off the origin, and
    /// `active` if it should start switched off.
    pub fn new(half_extents: Vec3, damage: f32) -> Self {
        Self {
            offset: Vec3::ZERO,
            half_extents,
            damage,
            active: true,
        }
    }
}

/// An oriented box that *receives* damage — the defending half of a hitbox/hurtbox pair.
///
/// Same nature as [`Hitbox`]: a local-frame box in metres, inert as far as collision detection
/// and the solver are concerned. The deliberate difference is that it carries no `active`
/// flag, so a hurtbox cannot be switched off in place — removing the component (or shrinking
/// it away) is the only way to make a region untouchable.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Hurtbox {
    /// Centre of the box relative to the entity's `Transform`, following the same local-frame
    /// placement rule as [`Hitbox::offset`].
    ///
    /// One component holds exactly one box, so splitting a body into head/torso/limb regions
    /// means one entity per region rather than a list on a single entity.
    pub offset: Vec3,
    /// Half-sizes along the local X/Y/Z axes in metres — the same convention, and the same
    /// absence of validation, as [`Hitbox::half_extents`].
    ///
    /// The default `(0.3, 0.5, 0.3)` describes a 0.6 × 1.0 × 0.6 m torso: notably taller than
    /// the small cube a `Hitbox` defaults to, which is the usual shape of the pair.
    pub half_extents: Vec3,
    /// Scales what a hit landing on this region costs: `1.0` (the default) is neutral, values
    /// above 1 mark a weak point such as a head, and `0.0` makes the region take nothing.
    ///
    /// Unclamped and unchecked — negatives are accepted and would flip the sign of the damage
    /// for any code that simply multiplies. Nothing in the engine applies it, so this
    /// convention is only as real as the game's own hit resolution makes it.
    pub damage_multiplier: f32,
}

impl Default for Hurtbox {
    fn default() -> Self {
        Self {
            offset: Vec3::ZERO,
            half_extents: Vec3::new(0.3, 0.5, 0.3),
            damage_multiplier: 1.0,
        }
    }
}

impl Hurtbox {
    /// Builds a neutral (`damage_multiplier` = `1.0`) hurtbox of the given size, centred on
    /// the entity's transform origin.
    ///
    /// The extents are stored verbatim — this is a plain constructor, not a validating one.
    /// The multiplier is the only field it fixes, so weak points are made by assigning
    /// `damage_multiplier` after construction.
    pub fn new(half_extents: Vec3) -> Self {
        Self {
            offset: Vec3::ZERO,
            half_extents,
            damage_multiplier: 1.0,
        }
    }
}

gizmo_core::impl_component!(Hitbox);
gizmo_core::impl_component!(Hurtbox);
