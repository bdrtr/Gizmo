//! A motor boat: thrust at the stern, a rudder, a keel — what floats a hull and steers it through
//! a [`FluidZone`](gizmo_physics_rigid::world::FluidZone).
//!
//! The water's own lift and drag are the fluid zone's ([`PhysicsWorld::fluid_zones`]): a body
//! lighter than the water floats in it at the right depth and is slowed by it. What the zone
//! does not know is that a boat is a boat — that it has a propeller under its stern, that its
//! hull resists going sideways far more than forwards, that it rolls back upright. A
//! [`BoatController`] adds those, from a throttle and a rudder, every physics step
//! ([`boat_controller_system`]):
//!
//! - **thrust**, along the hull's level forward, at the propeller — only while the propeller is
//!   under water, so a boat run aground stays there;
//! - **turning**, a yaw torque in proportion to the rudder and to the speed through the water
//!   (a rudder steers nothing standing still), and a yaw damping;
//! - **keel**, most of the sideways speed taken out, so the rudder turns the boat instead of
//!   sliding it; and a roll and pitch damping, the bilge keels' work.
//!
//! The controls follow the engine's steering convention ([`VehicleController::steering_input`]):
//! a positive rudder turns the bow right.
//!
//! Written straight into the [`Velocity`] as impulses, as the car model does, so a game's own
//! [`ExternalForce`](gizmo_physics_rigid::components::ExternalForce) — a gust of wind — still
//! adds to it.
//!
//! [`PhysicsWorld::fluid_zones`]: gizmo_physics_rigid::world::PhysicsWorld::fluid_zones
//! [`VehicleController::steering_input`]: crate::VehicleController::steering_input

use gizmo_core::component::IsDeleted;
use gizmo_core::query::Without;
use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::Transform;
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::world::PhysicsWorld;

/// What drives a boat — see the [module](self).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct BoatController {
    /// Throttle, −1 (full astern) … 1 (full ahead).
    pub throttle_input: f32,
    /// Rudder, −1 … 1; positive turns the bow right.
    pub rudder_input: f32,
    /// The propeller's push at full throttle, in newtons.
    pub max_thrust: f32,
    /// Where the propeller is, in the hull's own frame (forward −Z): under the stern.
    pub propeller: Vec3,
    /// Yaw torque per unit of rudder per m/s of speed, N·m·s/m.
    pub turn_per_speed: f32,
    /// The speed past which the rudder turns no harder, m/s.
    pub turn_speed_limit: f32,
    /// Yaw torque against the turning rate, N·m·s.
    pub yaw_damping: f32,
    /// Roll and pitch torque against their rates, N·m·s.
    pub roll_damping: f32,
    /// How much of the sideways momentum the keel takes out per second, as a multiple of the
    /// mass (2 — most of it within half a second).
    pub keel: f32,
}

impl BoatController {
    /// A boat pushed by `max_thrust` newtons from a propeller under the stern of a hull
    /// `half_length` long from its centre to the stern, with the turning and damping that a
    /// five-metre dinghy of about 400 kg wants (the `kayik` game's).
    #[must_use]
    pub fn new(max_thrust: f32, half_length: f32) -> Self {
        Self {
            throttle_input: 0.0,
            rudder_input: 0.0,
            max_thrust,
            propeller: Vec3::new(0.0, -0.25, half_length),
            turn_per_speed: 900.0,
            turn_speed_limit: 6.0,
            yaw_damping: 1500.0,
            roll_damping: 2500.0,
            keel: 2.0,
        }
    }

    /// Whether the propeller at `t` is in the water `world` holds — what thrust needs.
    #[must_use]
    pub fn propeller_in_water(&self, world: &PhysicsWorld, t: &Transform) -> bool {
        world.water_at(t.position + t.rotation * self.propeller).is_some_and(|w| w.depth > 0.0)
    }

    /// The force and torque on a hull at `t` moving at `velocity`, for one step: what
    /// [`boat_controller_system`] applies. `afloat` is whether the hull is in the water at all
    /// (the keel and the dampings work only there); `driving` whether the propeller is.
    #[must_use]
    pub fn forces(&self, t: &Transform, velocity: &Velocity, mass: f32, afloat: bool, driving: bool) -> (Vec3, Vec3) {
        let forward = t.rotation * Vec3::NEG_Z;
        let side = t.rotation * Vec3::X;
        let level_forward = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
        let level_side = Vec3::new(side.x, 0.0, side.z).normalize_or_zero();
        let (v, w) = (velocity.linear, velocity.angular);
        let mut force = Vec3::ZERO;
        let mut torque = Vec3::ZERO;
        if driving {
            let thrust = level_forward * (self.throttle_input.clamp(-1.0, 1.0) * self.max_thrust);
            force += thrust;
            torque += (t.rotation * self.propeller).cross(thrust);
        }
        if afloat {
            let speed = v.dot(level_forward).clamp(-self.turn_speed_limit, self.turn_speed_limit);
            // Positive rudder turns the bow right: a negative yaw about +Y.
            torque += Vec3::Y * (-self.rudder_input.clamp(-1.0, 1.0) * speed * self.turn_per_speed - w.y * self.yaw_damping);
            // Level: on a heeled boat the beam is tilted, and a tilted push would lift it.
            force -= level_side * (v.dot(level_side) * mass * self.keel);
            torque -= forward * (w.dot(forward) * self.roll_damping) + side * (w.dot(side) * self.roll_damping);
        }
        (force, torque)
    }
}

impl gizmo_core::component::Component for BoatController {}

/// Drives every [`BoatController`] one step of `dt` seconds through the world's water — see the
/// [module](self). A world without a `PhysicsWorld` has no water, and its boats do nothing.
pub fn boat_controller_system(world: &World, dt: f32) {
    let Some(physics) = world.get_resource::<PhysicsWorld>() else { return };
    if physics.fluid_zones.is_empty() {
        return;
    }
    // SAFETY: an exclusive system — the scheduler runs a `fn(&World, f32)` alone, so nothing else
    // holds a view of `Velocity` or `RigidBody` while this one does; `PhysicsWorld` is a resource,
    // not a component, and the read guard above aliases none of these.
    let mut boats = unsafe {
        world.query_filtered_unchecked::<(&BoatController, &Transform, &mut Velocity, &mut RigidBody), Without<IsDeleted>>()
    };
    for (boat, t, mut velocity, mut body) in boats.iter_mut() {
        if !body.is_dynamic() {
            continue;
        }
        let afloat = physics.water_at(t.position).is_some_and(|w| w.depth > 0.0)
            || physics.water_at(t.position - Vec3::Y * 0.5).is_some_and(|w| w.depth > 0.0);
        let driving = boat.propeller_in_water(&physics, t);
        let (force, torque) = boat.forces(t, &velocity, body.mass, afloat, driving);
        if force == Vec3::ZERO && torque == Vec3::ZERO {
            continue;
        }
        if boat.throttle_input != 0.0 || boat.rudder_input != 0.0 {
            body.wake_up();
        }
        velocity.linear += force * (body.inv_mass() * dt);
        velocity.angular += body.inv_world_inertia_tensor(t.rotation) * torque * dt;
        let (linear, angular) = body.mask_locked(velocity.linear, velocity.angular);
        velocity.linear = linear;
        velocity.angular = angular;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_math::Quat;
    use gizmo_physics_rigid::world::{FluidZone, ZoneShape};

    fn lake() -> PhysicsWorld {
        let mut p = PhysicsWorld::new();
        p.fluid_zones.push(FluidZone {
            shape: ZoneShape::Box { min: Vec3::new(-100.0, -50.0, -100.0), max: Vec3::new(100.0, 0.0, 100.0) },
            ..Default::default()
        });
        p
    }

    /// Full ahead pushes along the bow; a positive rudder at speed turns it right (negative yaw);
    /// the keel takes out sideways speed; nothing pushes a propeller out of the water.
    #[test]
    fn thrust_rudder_and_keel_push_the_right_way() {
        let mut boat = BoatController::new(9000.0, 1.6);
        let t = Transform::new(Vec3::ZERO);
        boat.throttle_input = 1.0;
        let (f, _) = boat.forces(&t, &Velocity::default(), 400.0, true, true);
        assert!(f.z < -8999.0 && f.x.abs() < 1e-3, "full ahead is −Z: {f:?}");
        boat.throttle_input = 0.0;
        boat.rudder_input = 1.0;
        let moving = Velocity::new(Vec3::new(0.0, 0.0, -4.0));
        let (_, tq) = boat.forces(&t, &moving, 400.0, true, true);
        assert!(tq.y < 0.0, "rudder right at speed yaws right (negative about +Y): {tq:?}");
        let (_, still) = boat.forces(&t, &Velocity::default(), 400.0, true, true);
        assert_eq!(still.y, 0.0, "a rudder steers nothing standing still");
        boat.rudder_input = 0.0;
        let sliding = Velocity::new(Vec3::new(3.0, 0.0, 0.0));
        let (keel, _) = boat.forces(&t, &sliding, 400.0, true, true);
        assert!((keel.x + 3.0 * 400.0 * 2.0).abs() < 1e-3, "the keel takes out sideways speed: {keel:?}");
        boat.throttle_input = 1.0;
        let (aground, _) = boat.forces(&t, &Velocity::default(), 400.0, false, false);
        assert_eq!(aground, Vec3::ZERO, "out of the water: no thrust, no keel");
    }

    /// Through the system: a boat on the lake with the throttle open gathers speed forwards and,
    /// with the rudder over, turns right; the same boat on land gets nothing.
    #[test]
    fn the_system_drives_a_boat_in_water_and_not_on_land() {
        let run = |at: Vec3, rudder: f32| {
            let mut world = World::new();
            world.insert_resource(lake());
            let mut boat = BoatController::new(9000.0, 1.6);
            boat.throttle_input = 1.0;
            boat.rudder_input = rudder;
            let mut body = RigidBody::new(400.0, false);
            body.calculate_box_inertia(1.6, 0.6, 3.2);
            let e = world.spawn_bundle((boat, Transform::new(at).with_rotation(Quat::IDENTITY), Velocity::default(), body));
            for _ in 0..60 {
                boat_controller_system(&world, 1.0 / 60.0);
            }
            *world.get::<Velocity>(e).unwrap()
        };
        let ahead = run(Vec3::new(0.0, 0.1, 0.0), 0.0);
        assert!(ahead.linear.z < -5.0, "one second at full throttle: {:?}", ahead.linear);
        let turning = run(Vec3::new(0.0, 0.1, 0.0), 1.0);
        assert!(turning.angular.y < -0.1, "rudder right turns right: {:?}", turning.angular);
        let land = run(Vec3::new(0.0, 5.0, 0.0), 1.0);
        assert_eq!(land.linear, Vec3::ZERO, "a boat on land goes nowhere");
    }
}
