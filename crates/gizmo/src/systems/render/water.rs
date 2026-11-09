//! The water a camera can be under, handed from the game's physics to the draw.
//!
//! Underwater fog is decided per drawn camera — the main view, a split-screen viewport, a
//! reflection probe's face — by asking which fluid zone holds it. The zones belong to the physics
//! world, which the draw used to reach into for that one point query, and which a render world
//! cannot hold: a whole simulation, not `Clone`, copied to answer "is this point in water". So the
//! frame's preparation hands the zones over as [`WaterVolumes`], a plain resource the draw reads
//! and an [extractor](gizmo_core::extract::Extractor) copies (`resource::<WaterVolumes>()`).

use crate::core::World;
use crate::physics::world::{water_at, FluidZone, PhysicsWorld, WaterSample};

/// The scene's fluid zones as the draw sees them — a copy of [`PhysicsWorld::fluid_zones`] taken
/// by [`prepare_frame`](super::prepare_frame), written only when the zones changed.
///
/// Without a `PhysicsWorld` the preparation leaves it alone, so a scene drawn with no physics (a
/// render world, a test) can put its own water here.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WaterVolumes {
    /// The zones, in the physics world's order.
    pub zones: Vec<FluidZone>,
}

impl WaterVolumes {
    /// The water at `p` — the same answer [`PhysicsWorld::water_at`] gives for the zones copied.
    #[must_use]
    pub fn water_at(&self, p: gizmo_math::Vec3) -> Option<WaterSample> {
        water_at(&self.zones, p)
    }
}

/// Brings [`WaterVolumes`] up to date with the physics world's zones. The resource is written
/// only when they differ, so an extractor copies it on the frames the water changed and no other.
pub(crate) fn publish_water_volumes(world: &mut World) {
    let fresh = {
        let Some(physics) = world.get_resource::<PhysicsWorld>() else {
            return;
        };
        let current = world.get_resource::<WaterVolumes>();
        if current.is_some_and(|w| w.zones == physics.fluid_zones) {
            return;
        }
        WaterVolumes { zones: physics.fluid_zones.clone() }
    };
    world.insert_resource(fresh);
}
