use crate::app::{App, Plugin};
use crate::physics::world::PhysicsWorld;

use crate::math::Vec3;

/// Gizmo Engine Fizik Eklentisi (Plugin).
/// Eklendiğinde fizik dünyasını (PhysicsWorld) başlatır.
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

impl<State: 'static> Plugin<State> for PhysicsPlugin {
    fn build(&self, app: &mut App<State>) {
        println!("[Plugin] PhysicsPlugin yükleniyor (Yerçekimi: {:?})...", self.gravity);
        app.world.insert_resource(PhysicsWorld::new().with_gravity(self.gravity));
        // Not: İleride `physics_step_system` buraya bir sistem (Schedule) olarak eklenebilir.
    }
}
