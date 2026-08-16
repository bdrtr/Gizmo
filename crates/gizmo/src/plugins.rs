use crate::app::Plugin;
#[cfg(feature = "physics")]
use gizmo_physics_rigid::world::PhysicsWorld;

use crate::math::Vec3;

/// Gizmo Engine Physics Plugin.
/// When added, it initializes the physics world (PhysicsWorld).
#[non_exhaustive]
#[cfg(feature = "physics")]
pub struct PhysicsPlugin {
    pub gravity: Vec3,
}

#[cfg(feature = "physics")]
impl Default for PhysicsPlugin {
    fn default() -> Self {
        Self {
            gravity: Vec3::new(0.0, -9.81, 0.0),
        }
    }
}

#[cfg(feature = "physics")]
impl PhysicsPlugin {
    /// Creates a new PhysicsPlugin with the default gravity.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the gravity vector (chainable).
    pub fn with_gravity(mut self, gravity: Vec3) -> Self {
        self.gravity = gravity;
        self
    }
}

#[cfg(feature = "physics")]
impl Plugin for PhysicsPlugin {
    fn build(&self, app: &mut dyn gizmo_app::AppLike) {
        let app = app.parts_mut();
        tracing::info!(
            "[Plugin] PhysicsPlugin yükleniyor (Yerçekimi: {:?})...",
            self.gravity
        );
        app.world
            .insert_resource(PhysicsWorld::new().with_gravity(self.gravity));
        // Run the physics step automatically at the app's fixed timestep (the
        // `PhysicsTime` accumulator loop that also drives `TransformPlugin`), so
        // callers don't hand-call `cpu_physics_step_system` every frame. Labelled
        // so transform systems can order themselves after it if both are added.
        app.schedule.add_di_system(
            gizmo_core::system::SystemConfig::new(Box::new(
                crate::systems::physics::PhysicsStepSystem,
            ))
            .label("physics_step"),
        );
        // Resolve any `AutoBoxCollider` markers (box collider sized from Transform.scale)
        // strictly BEFORE the physics step reads them, so a marked body never takes its
        // first step with the placeholder unit box. Registered here (same plugin, same
        // default phase) so the `physics_step` label is guaranteed to exist and the
        // `.before` edge actually binds.
        app.schedule.add_di_system(
            gizmo_core::system::SystemConfig::new(Box::new(
                crate::systems::auto_collider::AutoBoxColliderSystem,
            ))
            .label("auto_box_collider")
            .before("physics_step"),
        );
    }
}

/// The plugin that starts the Transform (hierarchy and synchronization) systems.
#[cfg(feature = "physics")]
pub struct TransformPlugin;

#[cfg(feature = "physics")]
impl Plugin for TransformPlugin {
    fn build(&self, app: &mut dyn gizmo_app::AppLike) {
        let app = app.parts_mut();
        // Per-frame, not per fixed step — and this is an ordering fix as much as a cost one.
        //
        // These ran in the fixed-timestep schedule, so they propagated `0..N` times per
        // rendered frame while the result is only ever consumed once, at draw. The comment
        // on `PhysicsPlugin`'s `physics_step` label says transform systems "can order
        // themselves after it", but no such edge was ever wired — so within a single fixed
        // step the order was whatever the batcher chose.
        //
        // The update schedule runs after *every* fixed step of the frame (see
        // `gizmo_app::frame::run_fixed_and_update`), so "transforms propagate after physics"
        // is now structural rather than dependent on a label edge that did not exist. It is
        // also after the per-frame update systems, which is what a camera moved by
        // `FpsLookSystem` needs.
        //
        // `default_render_pass` still calls `ensure_global_transforms` immediately before
        // drawing. That stays: it is the safety net for a custom `App` that never registers
        // this plugin, and it is what backfills a `GlobalTransform` onto a freshly spawned
        // mesh. With this plugin registered the propagation is simply already current.
        app.update_schedule.add_di_system(
            gizmo_core::system::SystemConfig::new(Box::new(
                crate::systems::transform::TransformSyncSystem,
            ))
            .label("transform_sync"),
        );
        app.update_schedule.add_di_system(
            gizmo_core::system::SystemConfig::new(Box::new(
                crate::systems::transform::TransformPropagateSystem,
            ))
            .label("transform_propagate")
            .after("transform_sync"),
        );
    }
}
