//! Drawing physics between its steps: a body is drawn where it is `alpha` of the way from the pose
//! before the last fixed step to the pose after it, so a game stepping physics at 60 Hz on a 144 Hz
//! screen moves smoothly instead of holding each pose for two or three frames.
//!
//! Measured before this existed (`gizmo_core::time`'s tests, `hold_lengths`): at 144 Hz a pose was
//! held 3, 2, 3, 2, 2… frames, so a body moving at a constant speed advanced by amounts 50 % apart on
//! neighbouring frames. The engine computed the coefficient (`PhysicsTime::alpha`) and nothing read
//! it, because nothing kept the pose before the step to blend from.
//!
//! **What is kept, and where.** [`InterpolationHistory`], a resource, holds each moving body's pose
//! before and after the last step, by entity id — a slot per id, as `MotionHistory` keeps model
//! matrices — and the coefficient. Not a component: adding one to every body would move them all to
//! new archetypes and change the order the physics step meets them, and with it the simulation.
//! The driver records the poses ([`begin_step`](InterpolationHistory::begin_step) and
//! [`end_step`](InterpolationHistory::end_step) around each step) and the coefficient
//! ([`set_alpha`](InterpolationHistory::set_alpha)): `PhysicsPlugin` does with systems around its
//! `physics_step`, and `FixedPhysics::advance` does in its loop. `simple_scene_update` steps physics
//! by the frame's own time, so what it draws is what it simulated, and keeps no history.
//!
//! **What is drawn.** [`render_overrides`] works out each recorded body's drawn world matrix — its
//! parent's as it is, times its own pose blended — and its descendants moved with it, by the same
//! change (a car's wheels, a piece's parts: the hierarchy is not propagated a second time). The draw
//! writes those into `GlobalTransform` for the draw alone and puts the old ones back after
//! ([`apply_overrides`], [`restore`]), on the render world's copy or, for `default_render_pass`, on
//! the world itself. `Transform` is never touched, and nothing the simulation reads ever holds a
//! blended pose: the state hash is what it was.
//!
//! **A body moved outside the step** — a teleport in the update, a rollback — has a `Transform` that
//! is not the pose recorded after the last step; it is drawn where it is, unblended, until the next
//! step records it again.

use gizmo_core::{Entity, World};
use gizmo_math::{Mat4, Quat, Vec3};
use gizmo_physics_core::components::{GlobalTransform, Transform};

/// A body's pose as the history compares and blends it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Pose {
    position: Vec3,
    rotation: Quat,
    scale: Vec3,
}

impl Pose {
    fn of(t: &Transform) -> Self {
        Self { position: t.position, rotation: t.rotation, scale: t.scale }
    }

    fn blend(self, to: Pose, alpha: f32) -> Mat4 {
        Mat4::from_scale_rotation_translation(
            self.scale.lerp(to.scale, alpha),
            self.rotation.slerp(to.rotation, alpha),
            self.position.lerp(to.position, alpha),
        )
    }
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    generation: u32,
    /// The step this entry was last written in; an entry not written in the last step is stale.
    step: u64,
    before: Pose,
    after: Pose,
}

/// Each moving body's pose before and after the last physics step, and how far into the next step
/// the frame is — see the [module](self).
#[derive(Debug, Default)]
pub struct InterpolationHistory {
    entries: Vec<Option<Entry>>,
    /// Steps begun; an entry is current when its `step` is this.
    step: u64,
    alpha: f32,
}

impl InterpolationHistory {
    /// Records every moving body's pose before a step. Call it before each step.
    pub fn begin_step(&mut self, world: &World) {
        self.step += 1;
        let step = self.step;
        for (entity, transform) in moving_bodies(world) {
            let slot = entity.id() as usize;
            if slot >= self.entries.len() {
                self.entries.resize(slot + 1, None);
            }
            let pose = Pose::of(&transform);
            self.entries[slot] = Some(Entry { generation: entity.generation(), step, before: pose, after: pose });
        }
    }

    /// Records every moving body's pose after a step. Call it after each step.
    pub fn end_step(&mut self, world: &World) {
        let step = self.step;
        for (entity, transform) in moving_bodies(world) {
            if let Some(Some(e)) = self.entries.get_mut(entity.id() as usize) {
                if e.generation == entity.generation() && e.step == step {
                    e.after = Pose::of(&transform);
                }
            }
        }
    }

    /// How far into the next step this frame is drawn, 0..1: the driver's leftover time over its step.
    pub fn set_alpha(&mut self, alpha: f32) {
        self.alpha = if alpha.is_finite() { alpha.clamp(0.0, 1.0) } else { 0.0 };
    }

    /// The coefficient last set.
    #[must_use]
    pub fn alpha(&self) -> f32 {
        self.alpha
    }

    fn current(&self, entity: Entity) -> Option<&Entry> {
        self.entries
            .get(entity.id() as usize)?
            .as_ref()
            .filter(|e| e.generation == entity.generation() && e.step == self.step)
    }
}

/// The bodies the physics step moves: a `RigidBody` that is not static, drawn and not pooled.
fn moving_bodies(world: &World) -> Vec<(Entity, Transform)> {
    use gizmo_physics_rigid::components::RigidBody;
    world
        .query_filtered::<(Entity, &RigidBody, &Transform), (
            gizmo_core::query::Without<gizmo_core::pool::Pooled>,
            gizmo_core::query::Without<gizmo_core::component::IsDeleted>,
        )>()
        .iter()
        .filter(|(_, body, _)| !body.is_static())
        .map(|(e, _, t)| (e, *t))
        .collect()
}

/// The world matrix each recorded body, and everything under it, is drawn with this frame. Empty
/// without an [`InterpolationHistory`].
#[must_use]
pub fn render_overrides(world: &World) -> Vec<(Entity, Mat4)> {
    let Some(history) = world.get_resource::<InterpolationHistory>() else { return Vec::new() };
    let alpha = history.alpha;
    let transforms = world.query::<&Transform>();
    let globals = world.query::<&GlobalTransform>();
    let children = world.query::<&gizmo_core::component::Children>();
    let mut out = Vec::new();
    for (entity, _) in moving_bodies(world) {
        let Some(entry) = history.current(entity) else { continue };
        let (Some(t), Some(g)) = (transforms.get(entity), globals.get(entity)) else { continue };
        // Moved since the step recorded it: drawn where it is.
        if Pose::of(t) != entry.after || entry.before == entry.after {
            continue;
        }
        let local = Mat4::from_scale_rotation_translation(t.scale, t.rotation, t.position);
        if local.determinant().abs() < 1e-12 || g.matrix.determinant().abs() < 1e-12 {
            continue;
        }
        let parent = g.matrix * local.inverse();
        let drawn = parent * entry.before.blend(entry.after, alpha);
        let delta = drawn * g.matrix.inverse();
        out.push((entity, drawn));
        // Its descendants move with it, by the same change; a descendant that is a body of its own
        // is drawn by its own entry.
        let mut stack: Vec<Entity> = children.get(entity).map(|c| c.0.clone()).unwrap_or_default();
        let mut seen = std::collections::HashSet::new();
        while let Some(child) = stack.pop() {
            if !seen.insert(child) || history.current(child).is_some() {
                continue;
            }
            if let Some(cg) = globals.get(child) {
                out.push((child, delta * cg.matrix));
            }
            if let Some(c) = children.get(child) {
                stack.extend(c.0.iter().copied());
            }
        }
    }
    out
}

/// Writes `overrides` into the world's `GlobalTransform`s and returns what they replaced, for
/// [`restore`] after the draw.
pub fn apply_overrides(world: &mut World, overrides: &[(Entity, Mat4)]) -> Vec<(Entity, Mat4)> {
    let mut replaced = Vec::with_capacity(overrides.len());
    let mut globals = world.query_mut::<&mut GlobalTransform>();
    for (entity, matrix) in overrides {
        if let Some(mut g) = globals.get_mut(*entity) {
            replaced.push((*entity, g.matrix));
            g.matrix = *matrix;
        }
    }
    replaced
}

/// Puts back what [`apply_overrides`] replaced.
pub fn restore(world: &mut World, replaced: &[(Entity, Mat4)]) {
    let mut globals = world.query_mut::<&mut GlobalTransform>();
    for (entity, matrix) in replaced {
        if let Some(mut g) = globals.get_mut(*entity) {
            g.matrix = *matrix;
        }
    }
}

/// `PhysicsPlugin`'s: records the poses before its step.
pub struct InterpolationBeginSystem;

impl gizmo_core::system::System for InterpolationBeginSystem {
    fn access_info(&self) -> gizmo_core::system::AccessInfo {
        let mut info = gizmo_core::system::AccessInfo::new();
        info.is_exclusive = true;
        info
    }

    fn run(&mut self, world: &World, _dt: f32) {
        if let Some(mut history) = world.get_resource_mut::<InterpolationHistory>() {
            history.begin_step(world);
        }
    }
}

/// `PhysicsPlugin`'s: records the poses after its step.
pub struct InterpolationEndSystem;

impl gizmo_core::system::System for InterpolationEndSystem {
    fn access_info(&self) -> gizmo_core::system::AccessInfo {
        let mut info = gizmo_core::system::AccessInfo::new();
        info.is_exclusive = true;
        info
    }

    fn run(&mut self, world: &World, _dt: f32) {
        if let Some(mut history) = world.get_resource_mut::<InterpolationHistory>() {
            history.end_step(world);
        }
    }
}

/// `PhysicsPlugin`'s, once a frame after the fixed steps: the frame's coefficient,
/// `PhysicsTime::alpha`.
pub struct InterpolationAlphaSystem;

impl gizmo_core::system::System for InterpolationAlphaSystem {
    fn access_info(&self) -> gizmo_core::system::AccessInfo {
        let mut info = gizmo_core::system::AccessInfo::new();
        info.is_exclusive = true;
        info
    }

    fn run(&mut self, world: &World, _dt: f32) {
        let alpha = world.get_resource::<gizmo_core::time::PhysicsTime>().map(|t| t.alpha());
        if let (Some(alpha), Some(mut history)) = (alpha, world.get_resource_mut::<InterpolationHistory>()) {
            history.set_alpha(alpha);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simple::{sync_transforms, FixedPhysics};
    use gizmo_core::hierarchy::HierarchyExt;
    use gizmo_physics_core::components::Collider;
    use gizmo_physics_rigid::components::{RigidBody, Velocity};
    use gizmo_physics_rigid::world::PhysicsWorld;

    /// A world with no gravity and one body moving at `speed` m/s along X.
    fn moving_body(speed: f32) -> (World, Entity) {
        let mut world = World::new();
        world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::ZERO));
        let body = world.spawn();
        world.add_component(body, Transform::new(Vec3::new(0.0, 5.0, 0.0)));
        world.add_component(body, GlobalTransform::default());
        world.add_component(body, RigidBody::new(1.0, false));
        world.add_component(body, Velocity { linear: Vec3::new(speed, 0.0, 0.0), ..Default::default() });
        world.add_component(body, Collider::sphere(0.5));
        (world, body)
    }

    /// Where `entity` is drawn: its override, or its `GlobalTransform` when it has none.
    fn drawn_x(world: &World, entity: Entity) -> f32 {
        render_overrides(world)
            .iter()
            .find(|(e, _)| *e == entity)
            .map(|(_, m)| m.w_axis.x)
            .unwrap_or_else(|| world.get::<GlobalTransform>(entity).unwrap().matrix.w_axis.x)
    }

    /// How much the per-frame step varies: the largest and smallest frame-to-frame advance.
    fn spread(xs: &[f32]) -> (f32, f32) {
        let steps: Vec<f32> = xs.windows(2).map(|w| w[1] - w[0]).collect();
        (steps.iter().copied().fold(f32::MIN, f32::max), steps.iter().copied().fold(f32::MAX, f32::min))
    }

    /// **At 144 Hz a body stepped at 60 Hz is drawn moving evenly.** Its `Transform` holds each step's
    /// pose for two or three frames and then jumps a whole step; the drawn pose advances by the same
    /// amount every frame.
    #[test]
    fn a_body_stepped_at_60_hz_is_drawn_evenly_at_144() {
        let (mut world, body) = moving_body(6.0);
        let mut physics = FixedPhysics::new(60);
        let (mut raw, mut drawn) = (Vec::new(), Vec::new());
        for _ in 0..288 {
            physics.advance(&mut world, 1.0 / 144.0);
            sync_transforms(&mut world);
            raw.push(world.get::<Transform>(body).unwrap().position.x);
            drawn.push(drawn_x(&world, body));
        }
        let (raw, drawn) = (&raw[30..], &drawn[30..]);
        let ((raw_max, raw_min), (max, min)) = (spread(raw), spread(drawn));
        eprintln!("per-frame advance: raw {raw_min:.4}..{raw_max:.4}, drawn {min:.4}..{max:.4} (even: {:.4})", 6.0 / 144.0);
        assert!(raw_min < 1e-6 && raw_max > 0.09, "the raw pose should hold and jump: {raw_min}..{raw_max}");
        let even = 6.0 / 144.0;
        assert!((max - even).abs() < even * 0.05 && (min - even).abs() < even * 0.05, "drawn unevenly: {min}..{max} against {even}");
    }

    /// The same through `PhysicsPlugin`, the fixed schedule and `PhysicsTime`.
    #[test]
    fn the_physics_plugin_draws_a_body_evenly_at_144() {
        let (world, body) = moving_body(6.0);
        let mut run = crate::simple::HeadlessFrames::new(world).add_plugin(crate::plugins::PhysicsPlugin::new().with_gravity(Vec3::ZERO));
        let mut drawn = Vec::new();
        for _ in 0..288 {
            run.frame(1.0 / 144.0, |w, _, _, _| sync_transforms(w));
            drawn.push(drawn_x(&run.world, body));
        }
        let (max, min) = spread(&drawn[30..]);
        let even = 6.0 / 144.0;
        eprintln!("plugin, per-frame advance: drawn {min:.4}..{max:.4} (even {even:.4})");
        assert!((max - even).abs() < even * 0.05 && (min - even).abs() < even * 0.05, "drawn unevenly: {min}..{max} against {even}");
    }

    /// A body moved outside the step — a teleport in the update — is drawn where it is, not swept
    /// from where the step left it.
    #[test]
    fn a_teleported_body_is_drawn_where_it_is() {
        let (mut world, body) = moving_body(6.0);
        let mut physics = FixedPhysics::new(60);
        for _ in 0..10 {
            physics.advance(&mut world, 1.0 / 144.0);
        }
        world.query_mut::<&mut Transform>().get_mut(body).unwrap().position = Vec3::new(100.0, 5.0, 0.0);
        sync_transforms(&mut world);
        assert!(render_overrides(&world).iter().all(|(e, _)| *e != body), "the teleported body was blended");
        assert_eq!(drawn_x(&world, body), 100.0);
    }

    /// What is under a body moves with it: a child is drawn by the body's change, the hierarchy not
    /// propagated again.
    #[test]
    fn a_bodys_child_is_drawn_with_it() {
        let (mut world, body) = moving_body(6.0);
        let child = world.spawn();
        world.add_component(child, Transform::new(Vec3::new(0.0, 1.0, 0.0)));
        world.add_component(child, GlobalTransform::default());
        world.add_child(body, child);
        let mut physics = FixedPhysics::new(60);
        for _ in 0..7 {
            physics.advance(&mut world, 1.0 / 144.0);
        }
        sync_transforms(&mut world);
        let overrides = render_overrides(&world);
        let body_at = overrides.iter().find(|(e, _)| *e == body).expect("the body is blended").1.w_axis;
        let child_at = overrides.iter().find(|(e, _)| *e == child).expect("its child moves with it").1.w_axis;
        assert!((child_at.x - body_at.x).abs() < 1e-5 && (child_at.y - body_at.y - 1.0).abs() < 1e-5, "{body_at} {child_at}");
    }

    /// The blended poses go in for the draw and back out after, exactly; `Transform` is never written.
    #[test]
    fn the_draw_puts_every_matrix_back() {
        let (mut world, body) = moving_body(6.0);
        let mut physics = FixedPhysics::new(60);
        for _ in 0..5 {
            physics.advance(&mut world, 1.0 / 144.0);
        }
        sync_transforms(&mut world);
        let (global, local) = (world.get::<GlobalTransform>(body).unwrap().matrix, *world.get::<Transform>(body).unwrap());
        let overrides = render_overrides(&world);
        assert!(!overrides.is_empty());
        let replaced = apply_overrides(&mut world, &overrides);
        assert_ne!(world.get::<GlobalTransform>(body).unwrap().matrix, global, "nothing was drawn blended");
        restore(&mut world, &replaced);
        assert_eq!(world.get::<GlobalTransform>(body).unwrap().matrix, global);
        assert_eq!(*world.get::<Transform>(body).unwrap(), local);
    }

    /// Keeping the history changes nothing the simulation does: stepped with it and without, a body
    /// ends where it ended, bit for bit.
    #[test]
    fn the_history_leaves_the_simulation_alone() {
        let run = |history: bool| {
            let (mut world, body) = moving_body(3.0);
            world.add_component(body, Velocity { linear: Vec3::new(3.0, 1.0, -2.0), angular: Vec3::new(0.3, 0.1, 0.2), ..Default::default() });
            if history {
                world.insert_resource(InterpolationHistory::default());
            }
            for _ in 0..120 {
                if history {
                    world.get_resource_mut::<InterpolationHistory>().unwrap().begin_step(&world);
                }
                crate::systems::cpu_physics_step_system(&world, 1.0 / 60.0);
                if history {
                    world.get_resource_mut::<InterpolationHistory>().unwrap().end_step(&world);
                }
            }
            let t = *world.get::<Transform>(body).unwrap();
            (t.position.to_array().map(f32::to_bits), t.rotation.to_array().map(f32::to_bits))
        };
        assert_eq!(run(true), run(false));
    }
}
