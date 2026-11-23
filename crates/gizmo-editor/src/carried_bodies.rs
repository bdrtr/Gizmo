//! What the editor moves along with an entity it moves: the rigid bodies under it.
//!
//! A rigid body's `Transform` is its world pose whatever its parent (docs/ENGINE.md §3 item 7):
//! the propagation takes a body as a root, so moving its parent does not move it. In a running game
//! that is the point — a box in a truck is carried by friction, not by the hierarchy — but a group
//! moved in the editor is expected to take everything in it along. So the editor's move paths (the
//! gizmo, the inspector's Transform rows) carry every body under a moved entity by the moved
//! entity's own change in the world: the body lands where it would have been drawn had it been a
//! child like any other.
//!
//! The walk keeps the rule of the facade's `gizmo::systems::world_matrix`, which the editor sits
//! below.

use gizmo_core::component::Parent;
use gizmo_core::entity::Entity;
use gizmo_core::World;
use gizmo_math::Mat4;
use gizmo_physics_core::Transform;
use gizmo_physics_rigid::components::RigidBody;

/// Deeper than any real hierarchy; past it the chain is taken for a cycle.
const MAX_DEPTH: usize = 1024;

fn local(t: &Transform) -> Mat4 {
    Mat4::from_scale_rotation_translation(t.scale, t.rotation, t.position)
}

fn is_body(world: &World, entity: Entity) -> bool {
    world.get::<RigidBody>(entity).is_some()
}

/// The world matrix of the frame `entity`'s `Transform` is written in: the identity for a root and
/// for a rigid body, its parent's world pose for anything else. `None` where the chain up cannot be
/// followed: an ancestor with no `Transform`, a parent that is dead, a cycle.
pub(crate) fn parent_frame(world: &World, entity: Entity) -> Option<Mat4> {
    let mut frame = Mat4::IDENTITY;
    let mut current = entity;
    for _ in 0..MAX_DEPTH {
        if is_body(world, current) {
            return Some(frame);
        }
        let Some(parent) = world.get::<Parent>(current).map(|p| p.0) else {
            return Some(frame);
        };
        if !world.is_alive(parent) {
            return None;
        }
        frame = local(world.get::<Transform>(parent)?) * frame;
        current = parent;
    }
    None
}

/// Whether `ancestor` is above `entity` in the hierarchy.
fn is_below(world: &World, entity: Entity, ancestor: Entity) -> bool {
    let mut current = entity;
    for _ in 0..MAX_DEPTH {
        let Some(parent) = world.get::<Parent>(current).map(|p| p.0) else {
            return false;
        };
        if parent == ancestor {
            return true;
        }
        if !world.is_alive(parent) {
            return false;
        }
        current = parent;
    }
    false
}

/// The rigid bodies anywhere under `moved`, other than those in `except` — what moves with it.
pub(crate) fn bodies_under(world: &World, moved: Entity, except: &[Entity]) -> Vec<Entity> {
    let mut found: Vec<Entity> = world
        .query::<(Entity, &RigidBody)>()
        .iter()
        .map(|(entity, _)| entity)
        .filter(|&body| body != moved && !except.contains(&body) && is_below(world, body, moved))
        .collect();
    found.sort_unstable();
    found
}

/// The new `Transform` of each body under `moved` when `moved`'s own goes from `before` to `after`
/// (`except` is left out — the entities moved by hand in the same edit), as `(body, now, then)`.
/// Nothing is written: the caller writes it, through the access it already holds. Empty when
/// `moved`'s frame cannot be followed or its pose before the edit was singular (a zero scale),
/// since no change in the world can be read from it.
pub(crate) fn carried(
    world: &World,
    moved: Entity,
    before: &Transform,
    after: &Transform,
    except: &[Entity],
) -> Vec<(Entity, Transform, Transform)> {
    if before == after {
        return Vec::new();
    }
    let Some(frame) = parent_frame(world, moved) else { return Vec::new() };
    let then = frame * local(before);
    if then.determinant().abs() <= f32::EPSILON {
        return Vec::new();
    }
    let change = frame * local(after) * then.inverse();
    bodies_under(world, moved, except)
        .into_iter()
        .filter_map(|body| {
            let now = *world.get::<Transform>(body)?;
            let (scale, rotation, position) = (change * local(&now)).to_scale_rotation_translation();
            let mut next = Transform::new(position).with_rotation(rotation.normalize()).with_scale(scale);
            next.update_local_matrix();
            Some((body, now, next))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_core::hierarchy::HierarchyExt;
    use gizmo_math::{Quat, Vec3};

    /// A group with a body two metres along it, and under the body its mesh: the group is turned a
    /// quarter about Y and moved. The body goes where it would have been drawn as a plain child,
    /// turned with the group; the mesh, local to the body, is not written.
    #[test]
    fn a_moved_group_carries_its_bodies_as_children() {
        let mut world = World::new();
        let group = world.spawn_bundle((Transform::new(Vec3::new(1.0, 0.0, 0.0)),));
        let body = world.spawn_bundle((Transform::new(Vec3::new(3.0, 0.0, 0.0)), RigidBody::new(1.0, true)));
        let mesh = world.spawn_bundle((Transform::new(Vec3::new(0.0, 0.5, 0.0)),));
        world.add_child(group, body);
        world.add_child(body, mesh);
        let before = *world.get::<Transform>(group).unwrap();
        let after = Transform::new(Vec3::new(1.0, 0.0, 5.0)).with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2));

        let moved = carried(&world, group, &before, &after, &[]);
        assert_eq!(moved.len(), 1, "the body and only the body: {moved:?}");
        let (who, then, now) = moved[0];
        assert_eq!(who, body);
        assert_eq!(then.position, Vec3::new(3.0, 0.0, 0.0));
        // Two metres along the group's X, which now points along −Z.
        assert!(now.position.abs_diff_eq(Vec3::new(1.0, 0.0, 3.0), 1e-5), "the body landed at {}", now.position);
        assert!((now.rotation * Vec3::X).abs_diff_eq(Vec3::NEG_Z, 1e-5), "turned with the group");
        assert!(now.scale.abs_diff_eq(Vec3::ONE, 1e-5));
    }

    /// Under a parent that is itself turned and moved, the change is read in the world: a group
    /// slid one metre along its parent's X slides the body one metre along that X too.
    #[test]
    fn the_change_is_read_in_the_world_through_the_parents() {
        let mut world = World::new();
        let top = world.spawn_bundle((Transform::new(Vec3::new(10.0, 0.0, 0.0)).with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)),));
        let group = world.spawn_bundle((Transform::new(Vec3::ZERO),));
        let body = world.spawn_bundle((Transform::new(Vec3::new(4.0, 1.0, 0.0)), RigidBody::new(1.0, true)));
        world.add_child(top, group);
        world.add_child(group, body);
        let before = *world.get::<Transform>(group).unwrap();
        let after = Transform::new(Vec3::X);
        let (_, _, now) = carried(&world, group, &before, &after, &[])[0];
        // The parent's X is the world's −Z.
        assert!(now.position.abs_diff_eq(Vec3::new(4.0, 1.0, -1.0), 1e-5), "the body landed at {}", now.position);
        assert!(now.rotation.abs_diff_eq(Quat::IDENTITY, 1e-5), "a slide turns nothing");
    }

    /// A body moved by hand in the same edit is not carried as well, a body under a body goes with
    /// the outer one, and nothing moves when the pose did not change.
    #[test]
    fn what_is_carried_and_what_is_not() {
        let mut world = World::new();
        let group = world.spawn_bundle((Transform::new(Vec3::ZERO),));
        let picked = world.spawn_bundle((Transform::new(Vec3::X), RigidBody::new(1.0, true)));
        let outer = world.spawn_bundle((Transform::new(Vec3::Y), RigidBody::new(1.0, true)));
        let inner = world.spawn_bundle((Transform::new(Vec3::Z), RigidBody::new(1.0, true)));
        let elsewhere = world.spawn_bundle((Transform::new(Vec3::splat(9.0)), RigidBody::new(1.0, true)));
        world.add_child(group, picked);
        world.add_child(group, outer);
        world.add_child(outer, inner);
        let before = *world.get::<Transform>(group).unwrap();
        let after = Transform::new(Vec3::splat(2.0));
        let who: Vec<Entity> = carried(&world, group, &before, &after, &[picked]).into_iter().map(|(e, _, _)| e).collect();
        let mut want = vec![outer, inner];
        want.sort_unstable();
        assert_eq!(who, want, "the outer body and the one under it, not the picked one nor one elsewhere ({elsewhere:?})");
        assert!(carried(&world, group, &before, &before, &[]).is_empty());
        // Moving a body carries the body under it.
        let o = *world.get::<Transform>(outer).unwrap();
        let moved = carried(&world, outer, &o, &Transform::new(Vec3::new(0.0, 3.0, 0.0)), &[]);
        assert_eq!(moved.len(), 1);
        assert!(moved[0].2.position.abs_diff_eq(Vec3::new(0.0, 2.0, 1.0), 1e-5), "{}", moved[0].2.position);
    }
}
