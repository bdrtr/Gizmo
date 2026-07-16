use crate::app::{App, Plugin};
use gizmo_physics_rigid::world::PhysicsWorld;

use crate::math::Vec3;

/// Gizmo Engine Fizik Eklentisi (Plugin).
/// Eklendiğinde fizik dünyasını (PhysicsWorld) başlatır.
#[non_exhaustive]
pub struct PhysicsPlugin {
    pub gravity: Vec3,
}

impl Default for PhysicsPlugin {
    fn default() -> Self {
        Self {
            gravity: Vec3::new(0.0, -9.81, 0.0),
        }
    }
}

impl PhysicsPlugin {
    /// Varsayılan yerçekimi ile yeni bir PhysicsPlugin oluşturur.
    pub fn new() -> Self {
        Self::default()
    }

    /// Yerçekimi vektörünü ayarlar (zincirlenebilir).
    pub fn with_gravity(mut self, gravity: Vec3) -> Self {
        self.gravity = gravity;
        self
    }
}

impl<State: 'static> Plugin<State> for PhysicsPlugin {
    fn build(&self, app: &mut App<State>) {
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

/// Transform (hiyerarşi ve senkronizasyon) sistemlerini başlatan eklenti.
pub struct TransformPlugin;

impl<State: 'static> Plugin<State> for TransformPlugin {
    fn build(&self, app: &mut App<State>) {
        // PostUpdate (veya Update sonu) gibi bir faz eklenebilir, şimdilik direkt ekleniyor.
        app.schedule.add_di_system(
            gizmo_core::system::SystemConfig::new(Box::new(
                crate::systems::transform::TransformSyncSystem,
            ))
            .label("transform_sync"),
        );
        app.schedule.add_di_system(
            gizmo_core::system::SystemConfig::new(Box::new(
                crate::systems::transform::TransformPropagateSystem,
            ))
            .label("transform_propagate")
            .after("transform_sync"),
        );
    }
}
