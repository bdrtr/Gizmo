//! A drawn wheel that follows a [`VehicleController`]'s wheel: where its strut puts it, turned by
//! the steering, rolling with the car.
//!
//! The controller's wheels are not entities — they are rays and spring state on the chassis — so
//! the picture of a wheel is something else's job. Every demo with a car did that job by hand,
//! each a dozen lines that read the strut length, the steering and the spin angle and wrote a
//! world transform. [`WheelVisual`] on the wheel's
//! entity says which of its vehicle's wheels it is — the vehicle being its nearest ancestor that
//! carries a [`VehicleController`], so nothing names an entity and a saved scene reopens whole —
//! and [`wheel_visual_system`] places it:
//!
//! - **where:** at the end of the strut, `attachment_local_pos + direction_local ·
//!   suspension_length` in the chassis frame — so it drops into a dip and tucks up over a bump;
//! - **turned:** by [`Wheel::steering_angle`](super::Wheel::steering_angle) about the chassis up axis (positive left);
//! - **rolling:** by [`Wheel::rotation_angle`](super::Wheel::rotation_angle) about the chassis's lateral axis, the way a wheel
//!   carrying the car forwards (−Z) turns.
//!
//! The entity need not be the chassis's own child: a wheel node deep inside a loaded model (the
//! chassis → the model's root, scaled and turned → the wheel) works, because the transform it is
//! given is worked out through every ancestor between it and the chassis. Its orientation and
//! scale in the chassis frame are taken the first time it is placed, as its rest: a model's wheel
//! keeps whatever axis and size its author gave it, and only its place, its steering and its spin
//! come from the controller.

use gizmo_core::component::Parent;
use gizmo_core::world::World;
use gizmo_core::Entity;
use gizmo_math::{Mat4, Quat, Vec3};
use gizmo_physics_core::Transform;

use super::VehicleController;

/// This entity is the picture of wheel [`wheel`](Self::wheel) of the [`VehicleController`] on
/// its nearest ancestor that has one — see the [module](self).
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct WheelVisual {
    /// Index into the vehicle's [`VehicleController::wheels`].
    pub wheel: usize,
    /// Its orientation and scale in the chassis frame at rest, taken the first time it is placed.
    #[serde(skip)]
    rest: Option<(Quat, Vec3)>,
}

impl WheelVisual {
    /// The picture of wheel `wheel` of the vehicle above it.
    #[must_use]
    pub fn new(wheel: usize) -> Self {
        Self { wheel, rest: None }
    }
}

impl gizmo_core::component::Component for WheelVisual {}

/// Deeper than any real hierarchy between a wheel and its chassis; past it the chain is taken for
/// a cycle, and the wheel is left where it is.
const MAX_DEPTH: usize = 64;

fn local(t: &Transform) -> Mat4 {
    Mat4::from_scale_rotation_translation(t.scale, t.rotation, t.position)
}

/// `entity`'s vehicle — its nearest ancestor with a [`VehicleController`] — and the transform from
/// the vehicle's frame to `entity`'s parent's: the local transforms of every ancestor between them,
/// composed. `None` when no ancestor is a vehicle.
fn vehicle_above(world: &World, entity: Entity) -> Option<(Entity, Mat4)> {
    let mut chain = Mat4::IDENTITY;
    let mut current = world.get::<Parent>(entity)?.0;
    for _ in 0..MAX_DEPTH {
        if world.get::<VehicleController>(current).is_some() {
            return Some((current, chain));
        }
        chain = local(world.get::<Transform>(current)?) * chain;
        current = world.get::<Parent>(current)?.0;
    }
    None
}

/// The pose of a wheel in its chassis's frame: at the end of its strut, steered, rolled, with
/// the orientation and scale `rest` it was drawn with.
#[must_use]
pub fn wheel_pose(wheel: &super::Wheel, rest: (Quat, Vec3)) -> Mat4 {
    let down = wheel.direction_local.try_normalize().unwrap_or(Vec3::NEG_Y);
    let at = wheel.attachment_local_pos + down * wheel.suspension_length;
    // Rolling forwards (−Z) the top of the wheel moves forwards: a turn about −X.
    let turn = Quat::from_rotation_y(wheel.steering_angle) * Quat::from_rotation_x(-wheel.rotation_angle) * rest.0;
    Mat4::from_scale_rotation_translation(rest.1, turn, at)
}

/// Places every [`WheelVisual`] — see the [module](self). A wheel with no vehicle above it, or
/// whose vehicle has no such wheel, is left where it is.
pub fn wheel_visual_system(world: &World, _dt: f32) {
    let wheels: Vec<(Entity, WheelVisual)> = world.query::<(Entity, &WheelVisual)>().iter().map(|(e, w)| (e, *w)).collect();
    for (entity, visual) in wheels {
        let Some((vehicle, chain)) = vehicle_above(world, entity) else { continue };
        let Some(wheel) = world.get::<VehicleController>(vehicle).and_then(|v| v.wheels.get(visual.wheel).cloned()) else {
            continue;
        };
        let rest = match visual.rest {
            Some(rest) => rest,
            None => {
                let Some(t) = world.get::<Transform>(entity).map(local) else { continue };
                let (scale, rotation, _) = (chain * t).to_scale_rotation_translation();
                // SAFETY: an exclusive system — the scheduler runs a `fn(&World, f32)` alone, and no
                // other view of `WheelVisual` is alive here (the list above was collected).
                if let Some(mut w) = unsafe { world.query_unchecked::<&mut WheelVisual>() }.get_mut(entity) {
                    w.rest = Some((rotation, scale));
                }
                (rotation, scale)
            }
        };
        let (scale, rotation, position) = (chain.inverse() * wheel_pose(&wheel, rest)).to_scale_rotation_translation();
        // SAFETY: as above; no other view of `Transform` is alive (the reads above have ended).
        if let Some(mut t) = unsafe { world.query_unchecked::<&mut Transform>() }.get_mut(entity) {
            t.position = position;
            t.rotation = rotation;
            t.scale = scale;
            t.update_local_matrix();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_core::component::Children;

    fn car() -> VehicleController {
        let mut car = VehicleController::four_wheeled(1.6, 2.4, 0.33);
        for w in &mut car.wheels {
            w.suspension_length = 0.4;
        }
        car
    }

    /// A wheel inside a scaled, turned model hangs at the end of its strut in the world, keeps the
    /// size and axis its model gave it, steers left for a positive angle and rolls the way the car
    /// goes; a wheel of no vehicle above it is left alone.
    #[test]
    fn a_wheel_follows_its_strut_through_a_turned_scaled_model() {
        let mut world = World::new();
        let mut controller = car();
        let vehicle = world.spawn_bundle((Transform::new(Vec3::new(5.0, 1.0, 0.0)), controller.clone()));
        // The model: half a turn round, four times bigger, as a loaded glTF root would be.
        let model = world.spawn_bundle((
            Transform::new(Vec3::new(0.0, -0.5, 0.0)).with_rotation(Quat::from_rotation_y(std::f32::consts::PI)).with_scale(Vec3::splat(4.0)),
            Parent(vehicle),
        ));
        // The front-left wheel node, at its place in the model, its axle along the model's X.
        let wheel = world.spawn_bundle((Transform::new(Vec3::new(0.2, 0.0, 0.3)), Parent(model), WheelVisual::new(0)));
        world.add_component(vehicle, Children(vec![model]));
        world.add_component(model, Children(vec![wheel]));

        let world_pos = |world: &World, e: Entity| {
            let mut m = local(world.get::<Transform>(e).unwrap());
            let mut cur = e;
            while let Some(p) = world.get::<Parent>(cur).map(|p| p.0) {
                m = local(world.get::<Transform>(p).unwrap()) * m;
                cur = p;
            }
            m
        };

        wheel_visual_system(&world, 0.0);
        let m = world_pos(&world, wheel);
        let want = Vec3::new(5.0, 1.0, 0.0) + controller.wheels[0].attachment_local_pos + Vec3::new(0.0, -0.4, 0.0);
        assert!(m.w_axis.truncate().abs_diff_eq(want, 1e-4), "the wheel is at {:?}, its strut ends at {want:?}", m.w_axis);
        let (scale, rotation, _) = m.to_scale_rotation_translation();
        assert!(scale.abs_diff_eq(Vec3::splat(4.0), 1e-4), "the model's size is kept: {scale:?}");
        assert!((rotation * Vec3::X).abs_diff_eq(Vec3::NEG_X, 1e-4), "the model's axle is kept: {:?}", rotation * Vec3::X);

        // Steered left and rolled forward a quarter turn.
        controller.wheels[0].steering_angle = 0.3;
        controller.wheels[0].rotation_angle = std::f32::consts::FRAC_PI_2;
        world.add_component(vehicle, controller.clone());
        wheel_visual_system(&world, 0.0);
        let (_, rotation, _) = world_pos(&world, wheel).to_scale_rotation_translation();
        // The axle (the model's X, at rest the world's −X) turned 0.3 rad left about +Y.
        let axle = rotation * Vec3::X;
        let want = Vec3::new(-0.3f32.cos(), 0.0, 0.3f32.sin());
        assert!(axle.abs_diff_eq(want, 1e-3), "steering left turns the axle to {want:?}, not {axle:?}");
        // A point on top of the tyre moves forwards (−Z) as it rolls forwards.
        let steer = Quat::from_rotation_y(0.3);
        let top = steer.inverse() * (rotation * (Quat::from_rotation_y(std::f32::consts::PI).inverse() * Vec3::Y));
        assert!(top.z < -0.99, "rolled forward a quarter, the top of the wheel faces forward: {top:?}");

        // Under no vehicle: untouched.
        let stray = world.spawn_bundle((Transform::new(Vec3::splat(9.0)), WheelVisual::new(1)));
        wheel_visual_system(&world, 0.0);
        assert_eq!(world.get::<Transform>(stray).unwrap().position, Vec3::splat(9.0));
    }
}
