use gizmo_math::{Quat, Vec3};

use serde::{Deserialize, Serialize};

/// An oriented box that *deals* damage while it is active — the attacking half of a
/// hitbox/hurtbox pair.
///
/// This is gameplay data, not collision geometry. A `Hitbox` is not a
/// [`Collider`](crate::components::Collider) and takes no part in broadphase, narrowphase or
/// the solver, so it generates no contacts and stops nothing from moving.
///
/// `gizmo-physics-dynamics`' `hit_detection_system` resolves the overlaps and reports each one as
/// a [`HitEvent`]; **what a hit costs stays with the game**, which reads those events and takes
/// the health off itself. A game that does not schedule that system is unaffected — these boxes
/// then do nothing at all, which is what they used to do unconditionally.
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
    /// This is the flag a move's active window drives — clearing it through startup and recovery
    /// is what keeps an attack from connecting outside its window. Nothing in *this* crate reads
    /// or writes it; `hit_detection_system` does both, for every hitbox owned by a fighter (see
    /// [`Hitbox::move_name`]). A hitbox with no fighter above it is left exactly as authored, so
    /// a trap or a projectile can drive its own.
    pub active: bool,
    /// Which move this box belongs to, matched against
    /// [`CombatMove::name`](crate::components::fighter::CombatMove::name).
    ///
    /// `None` — the default — means "every move": the box is live whenever its fighter is inside
    /// any move's active window. That is the right answer for a fighter with one hitbox and the
    /// wrong one the moment there are two, which is why the name exists: a jab's fist box and a
    /// kick's foot box tagged `"Jab"` and `"Roundhouse"` go live only for their own move. The
    /// engine's fight system deleted in `592bd6f` drove *every* box in a fighter's subtree from
    /// the active window, and that is the defect this field closes.
    ///
    /// Matched by exact string equality; a name that matches no move simply never goes live.
    #[serde(default)]
    pub move_name: Option<String>,
}

impl Default for Hitbox {
    fn default() -> Self {
        Self {
            offset: Vec3::ZERO,
            half_extents: Vec3::new(0.2, 0.2, 0.2),
            damage: 10.0,
            active: true,
            move_name: None,
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
            move_name: None,
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

/// Do an active hitbox and a hurtbox overlap, given the world pose of each one's entity?
///
/// The two volumes are boxes in their owners' local frames, so this places each at
/// `position + rotation * offset` with the owner's rotation and runs the fifteen-axis SAT sweep
/// ([`NarrowPhase::box_box_overlap`](crate::narrowphase::NarrowPhase::box_box_overlap)).
///
/// **Scale is ignored**, deliberately: the engine's own debug gizmo draws these boxes from
/// `position + rotation * offset` and `half_extents` alone, so honouring scale here would make the
/// tested volume disagree with the drawn one — and for a hit volume, what the author sees is the
/// contract. Author the extents in metres.
///
/// Says nothing about `active`, about who owns what, or about whether the hit is allowed; it is
/// the geometry question on its own.
pub fn hit_volumes_overlap(
    hitbox: &Hitbox,
    hitbox_pos: Vec3,
    hitbox_rot: Quat,
    hurtbox: &Hurtbox,
    hurtbox_pos: Vec3,
    hurtbox_rot: Quat,
) -> bool {
    crate::narrowphase::NarrowPhase::box_box_overlap(
        hitbox_pos + hitbox_rot.mul_vec3(hitbox.offset),
        hitbox_rot,
        hitbox.half_extents,
        hurtbox_pos + hurtbox_rot.mul_vec3(hurtbox.offset),
        hurtbox_rot,
        hurtbox.half_extents,
    )
}

/// One connected hit: an active [`Hitbox`] overlapped a [`Hurtbox`] belonging to someone else.
///
/// **The engine reports the hit; the game decides what it costs.** Nothing in this workspace
/// subtracts `damage` from anybody — reading these events and applying them is the game's, and
/// that is the line: overlap resolution is geometry and frame timing, whereas death, armour,
/// counter-hits, throws and friendly fire are the rules of a particular fighting game.
///
/// Delivered through `gizmo_core::event::Events<HitEvent>` if that resource exists; produced by
/// `gizmo-physics-dynamics`' `hit_detection_system`, which runs after the fight clock so the
/// active window it reads is this frame's.
///
/// A move connects with a given victim **once**: the attacker records who it has hit and the
/// record is cleared when the move ends or is cancelled ([`FighterController::start_move`]).
/// Without that a three-frame active window would report three hits.
#[derive(Debug, Clone)]
pub struct HitEvent {
    /// Entity id of the fighter that landed it — the owner of the hitbox, which is the entity
    /// carrying the [`FighterController`](crate::components::fighter::FighterController), not
    /// necessarily the entity the box itself sits on.
    pub attacker: u32,
    /// Entity id the active [`Hitbox`] is on: the fighter itself, or a child of it (a fist, a
    /// foot). Equal to `attacker` when the box is on the fighter.
    pub attacker_hitbox: u32,
    /// Entity id of the fighter that was hit, if the hurtbox belongs to one; otherwise the
    /// hurtbox's own entity. A hurtbox on something that is not a fighter still reports.
    pub victim: u32,
    /// Entity id the [`Hurtbox`] is on — the region that was struck, which is what makes
    /// head/torso/limb hits distinguishable.
    pub victim_hurtbox: u32,
    /// Damage this hit is worth: the **move's**
    /// [`FrameData::damage`](crate::components::fighter::FrameData::damage) scaled by the
    /// region's [`Hurtbox::damage_multiplier`].
    ///
    /// The move's number rather than [`Hitbox::damage`] because the move is what a game (or a
    /// Lua script, through `fighter.set_move`) actually sets per attack, while a box's own
    /// `damage` is for a hitbox with no fighter behind it — a trap, a projectile — which this
    /// system does not touch.
    pub damage: f32,
    /// Frames of stun the move inflicts, straight from its frame data: pass it to the victim's
    /// [`FighterController::apply_hitstun`](crate::components::fighter::FighterController::apply_hitstun).
    pub hitstun: u32,
    /// Frames of freeze the move inflicts on connect — conventionally applied to **both**
    /// fighters, which is what gives a blow its weight. Nothing here applies it.
    pub hitstop: u32,
    /// The move that landed, by name; empty for an unnamed move.
    pub move_name: String,
}

#[cfg(feature = "ecs")]
gizmo_core::impl_component!(Hitbox);
#[cfg(feature = "ecs")]
gizmo_core::impl_component!(Hurtbox);

#[cfg(test)]
mod tests {
    use super::*;

    /// The overlap test places each box by its owner's pose and its own offset — so two entities
    /// standing apart can still connect if the boxes reach, and two standing together can miss if
    /// they do not.
    #[test]
    fn the_offsets_are_what_decides_whether_a_punch_reaches() {
        let mut fist = Hitbox::new(Vec3::splat(0.2), 8.0);
        fist.offset = Vec3::new(0.0, 1.2, -0.6); // out in front, at chest height
        let torso = Hurtbox::new(Vec3::new(0.3, 0.5, 0.3));

        let attacker = Vec3::ZERO;
        let facing = Quat::IDENTITY; // -Z is forward

        // A defender 0.9 m in front, torso centred on its origin: the fist reaches.
        assert!(hit_volumes_overlap(
            &fist,
            attacker,
            facing,
            &torso,
            Vec3::new(0.0, 1.2, -0.9),
            Quat::IDENTITY
        ));

        // The same defender two metres away: nothing connects.
        assert!(!hit_volumes_overlap(
            &fist,
            attacker,
            facing,
            &torso,
            Vec3::new(0.0, 1.2, -2.0),
            Quat::IDENTITY
        ));

        // And with the attacker turned around, the fist points the other way and misses.
        assert!(!hit_volumes_overlap(
            &fist,
            attacker,
            Quat::from_rotation_y(std::f32::consts::PI),
            &torso,
            Vec3::new(0.0, 1.2, -0.9),
            Quat::IDENTITY
        ));
    }

    /// `move_name` defaults to "any move" so a fighter with a single hitbox needs no tagging.
    #[test]
    fn a_fresh_hitbox_belongs_to_every_move() {
        assert!(Hitbox::default().move_name.is_none());
        assert!(Hitbox::new(Vec3::splat(0.2), 5.0).move_name.is_none());
    }
}
