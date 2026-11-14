//! **The engine bends the limbs it is asked to.** [`apply_ik_chains_system`] solves every
//! [`IkChains`] on the skeleton beside it, once a frame, after the skeletal animation drivers and
//! before bone attachments — so a hand reaches for its target on top of whatever the clip is
//! doing, and a sword in that hand goes with it.
//!
//! Until 2026-11-14 the engine had the solver and a `TwoBoneIkChain` component that held solve
//! parameters, and nothing applied either: the component's docs told the caller to write the joint
//! rotations back themselves, nobody did, and a scene could not carry one.
//!
//! The animated pose (`Skeleton::local_poses`) is left as the animation wrote it, and the bend is
//! computed from it afresh every frame (`gizmo_animation::ik::apply_ik_chains`), so a weight
//! below one does not compound toward the target on a skeleton nothing animates. What is bent is
//! the drawn pose: `Skeleton::global_poses` and the skinning matrices.

use crate::core::World;
use crate::renderer::components::Skeleton;
use crate::renderer::ik::IkChains;
use gizmo_core::Entity;
use gizmo_physics_core::components::GlobalTransform;

/// The chains already said to name no bone, per entity — said once, not every frame.
#[derive(Default)]
struct UnresolvedIkChains(std::collections::HashMap<Entity, Vec<String>>);

/// Bends every skeleton that has [`IkChains`] by them and uploads the bent pose.
///
/// The chains' targets and poles are world points, mapped into the skeleton's model space by the
/// inverse of the entity's `GlobalTransform` (the identity when it has none), which is the space
/// its joints are posed in. A chain whose tip bone the skeleton does not have bends nothing and is
/// named in a warning, once per entity until the set of such chains changes.
pub fn apply_ik_chains_system(world: &mut World, queue: &wgpu::Queue) {
    let mut unresolved_now: Vec<(Entity, Vec<String>)> = Vec::new();
    let jobs: Vec<(Entity, IkChains, gizmo_math::Mat4)> = {
        let globals = world.query::<&GlobalTransform>();
        world
            .query::<(Entity, &IkChains)>()
            .iter()
            .filter(|(_, ik)| !ik.chains.is_empty())
            .map(|(e, ik)| (e, ik.clone(), globals.get(e).map_or(gizmo_math::Mat4::IDENTITY, |g| g.matrix.inverse())))
            .collect()
    };
    if jobs.is_empty() && world.get_resource::<UnresolvedIkChains>().is_none() {
        return;
    }
    {
        let mut skeletons = world.query_mut::<&mut Skeleton>();
        for (entity, ik, model_from_world) in &jobs {
            let Some(mut skeleton) = skeletons.get_mut(*entity) else { continue };
            let outcome = crate::renderer::ik::apply_ik_chains(&skeleton.hierarchy, &skeleton.local_poses, *model_from_world, &ik.chains);
            skeleton.global_poses = outcome.global_poses;
            crate::renderer::upload_skin_poses(&skeleton, queue);
            unresolved_now.push((*entity, outcome.unresolved.iter().map(|&i| ik.chains[i].tip_bone.clone()).collect()));
        }
    }
    if unresolved_now.iter().all(|(_, names)| names.is_empty()) && world.get_resource::<UnresolvedIkChains>().is_none() {
        return;
    }
    let mut said = world.get_resource_mut_or_default::<UnresolvedIkChains>();
    said.0.retain(|e, _| unresolved_now.iter().any(|(n, _)| n == e));
    for (entity, names) in unresolved_now {
        if names.is_empty() {
            said.0.remove(&entity);
        } else if said.0.get(&entity) != Some(&names) {
            tracing::warn!(entity = %entity, bones = ?names, "[IK] chains whose tip bone the skeleton has not got (or that has no two parents) bend nothing");
            said.0.insert(entity, names);
        }
    }
}
