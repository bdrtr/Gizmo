use gizmo_math::Vec3;
use serde::{Deserialize, Serialize};

/// Tunables and per-step state for a **kinematic** character — one that walks, jumps, climbs
/// steps and swims by being moved directly, instead of by having forces applied to it.
///
/// The component itself is inert; the movement is performed by the kinematic character step
/// in `gizmo-physics-dynamics` (`update_character`), which reads these fields and writes the
/// entity's `Transform` and `Velocity`. Two things are worth knowing before tuning it. First,
/// a character driven this way should not also be a *dynamic* rigid body: the rigid-body
/// pipeline and this controller would each apply gravity. Second, not every field is read by
/// the step — `speed` and `jump_buffer_time` are values the game reads and acts on (see their
/// own docs), which is why changing them alone appears to do nothing.
///
/// Four fields are `#[serde(skip)]` (`is_grounded`, `coyote_timer`, `jump_buffer_timer`,
/// `is_submerged`), so a scene reload restores a character with no coyote grace, no buffered
/// jump, and neither grounded nor submerged until its first step re-derives them; the tuning
/// fields round-trip intact.
///
/// Units are SI throughout — lengths in metres, times in seconds, angles in radians — but
/// the derived unit differs per field (m/s, m/s², per-second rates), so read each field's
/// own doc rather than assuming.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharacterController {
    /// Nominal ground movement speed in m/s (`5.0` by default).
    ///
    /// Read by the game, not by the character step, which moves the body at
    /// `target_velocity` instead. The usual idiom in the input handler —
    /// `kcc.target_velocity = dir.normalize_or_zero() * kcc.speed` — is what ties the two
    /// together; without it this field has no effect at all.
    pub speed: f32,
    /// Upward launch speed in m/s applied the moment a jump fires (`5.0` by default).
    ///
    /// It is *assigned* to the vertical velocity rather than added as an impulse, so a jump
    /// taken during a fall is not weakened by the downward speed already accumulated. Nothing
    /// scales it by mass — a kinematic character has none as far as this controller cares.
    pub jump_speed: f32,
    /// Downward acceleration in m/s² (`9.81` by default), given as a positive **magnitude**
    /// rather than a signed vector: it is subtracted from the vertical velocity, so a negative
    /// value here makes the character fall upwards.
    ///
    /// This is the character's own gravity, unrelated to the rigid-body world's, and it is
    /// integrated only while `is_grounded` is false and only on land — while submerged,
    /// `buoyancy` and `water_drag` take over entirely.
    pub gravity: f32,
    /// Steepest ground still considered walkable, in **radians** measured from world +Y
    /// (45° by default).
    ///
    /// Ground within this angle carries the character normally; anything steeper makes it
    /// slide at `slope_slide_speed`. The same threshold classifies obstacles: a surface run
    /// into head-on counts as a wall — and so as a step-climb candidate — exactly when its
    /// normal exceeds this angle. `0.0` leaves only perfectly flat ground walkable, and π/2 or
    /// more makes even vertical faces count as walkable ground.
    pub max_slope_angle: f32, // in radians
    /// Speed in m/s at which the character slides down ground steeper than `max_slope_angle`
    /// (`10.0` by default).
    ///
    /// Added to the desired movement along the downslope direction rather than replacing it,
    /// so input still steers during a slide. `0.0` disables sliding, leaving the character
    /// standing on the steep face.
    pub slope_slide_speed: f32,
    /// Roughly the tallest ledge the character climbs onto instead of being stopped by, in
    /// metres (`0.3` by default).
    ///
    /// Climbing is attempted only while grounded and only when this is greater than `0.0`, so
    /// `0.0` turns every kerb into a wall. The probe is cast from around this height above the
    /// feet and the ledge top must itself be walkable under `max_slope_angle`, so treat the
    /// limit as approximate — the collider's own radius shifts it — rather than an exact
    /// cutoff.
    pub step_height: f32,

    /// Whether the character was resting on something at the end of the last step.
    ///
    /// An **output**, not a request: each step recomputes it (from a downward ray on land,
    /// forced to `false` while swimming) before acting on it, so a value written by game code
    /// is overwritten rather than obeyed. `#[serde(skip)]`.
    #[serde(skip)]
    pub is_grounded: bool,

    /// Desired velocity for this step, in **world space**, m/s — the movement request that
    /// drives the whole controller.
    ///
    /// On land only X and Z are honoured; vertical motion comes from `gravity` and
    /// `jump_speed`, so a non-zero `y` will not lift the character — though it does still
    /// count toward the vector's length, which the on-ground slope projection preserves, and
    /// so inflates the resulting horizontal speed. While submerged all three components are
    /// used, which is what lets a swimmer aim up and down. It is a velocity, not a per-step
    /// displacement — the step applies `dt` itself — and it persists until overwritten, so it
    /// must be set back to `Vec3::ZERO` to stop.
    pub target_velocity: Vec3, // Desired movement from input

    /// Grace period in seconds, after walking off a ledge, during which a jump still works
    /// (`0.1` s by default).
    ///
    /// The *reload* value for `coyote_timer`, refilled on every grounded step; raising it
    /// mid-air changes nothing until the character next touches ground. `0.0` removes the
    /// grace, allowing a jump only on a step that found ground.
    pub coyote_time: f32,
    /// Seconds of coyote grace remaining: refilled to `coyote_time` while grounded, counted
    /// down by `dt` while airborne, and zeroed when a jump fires so one launch cannot be spent
    /// twice.
    ///
    /// Not clamped at zero — it keeps going negative for as long as the character stays
    /// airborne, so only `> 0.0` is meaningful. `#[serde(skip)]`.
    #[serde(skip)]
    pub coyote_timer: f32,

    /// How long, in seconds, a jump press should stay queued while the character cannot yet
    /// jump (`0.1` s by default).
    ///
    /// Like `speed`, this is for the game rather than the step, which never reads it. It is
    /// the value an input handler copies into `jump_buffer_timer` on a jump press.
    pub jump_buffer_time: f32,
    /// Seconds left on a queued jump request; any value `> 0.0` means "jump at the first
    /// opportunity".
    ///
    /// Writing it is how a jump is actually requested: the step fires the jump when this and
    /// `coyote_timer` are both positive, then zeroes both. It is decremented by `dt` only
    /// while still positive, so an expired request settles just below zero instead of running
    /// away — it simply never fires. `#[serde(skip)]`, so a queued jump does not survive a
    /// save/load.
    #[serde(skip)]
    pub jump_buffer_timer: f32,

    // ── Su / yüzme ──────────────────────────────────────────
    /// Net upward lift acceleration while submerged (m/s²). 0 = neutral buoyancy, >0 rises to the
    /// surface; together with drag it settles at a terminal ascent speed.
    pub buoyancy: f32,
    /// Linear drag coefficient in water (each step `vel *= 1 - water_drag*dt`). Makes water viscous.
    pub water_drag: f32,
    /// How fast the swim thrust approaches `target_velocity` (water responsiveness).
    pub swim_acceleration: f32,
    /// Runtime: is the character currently inside a water volume (swim mode active). Not serialised.
    #[serde(skip)]
    pub is_submerged: bool,
}

impl Default for CharacterController {
    fn default() -> Self {
        Self {
            speed: 5.0,
            jump_speed: 5.0,
            gravity: 9.81,
            max_slope_angle: 45.0_f32.to_radians(),
            slope_slide_speed: 10.0,
            step_height: 0.3,
            is_grounded: false,
            target_velocity: Vec3::ZERO,
            coyote_time: 0.1,
            coyote_timer: 0.0,
            jump_buffer_time: 0.1,
            jump_buffer_timer: 0.0,
            buoyancy: 2.0,
            water_drag: 2.0,
            swim_acceleration: 8.0,
            is_submerged: false,
        }
    }
}

#[cfg(feature = "ecs")]
gizmo_core::impl_component!(CharacterController);
