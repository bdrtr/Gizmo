//! Building a glTF scene's nodes into entities: what [`Commands::spawn_gltf`] does with a file it
//! has just loaded, and what [`build_requested_gltf_models`] does for a `GltfModel` placed by name.
//! Render-only — a model's meshes need nothing from the physics feature — which is why it is not in
//! `spawner` (render *and* physics).
//!
//! [`Commands::spawn_gltf`]: crate::spawner::Commands::spawn_gltf

use gizmo_core::{Entity, EntityName, World};
use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::Transform;
use gizmo_renderer::components::{Material, MeshRenderer};
use gizmo_renderer::Renderer;

/// The joints of a model's first skin — the one its root's `Skeleton` is — by glTF node index:
/// which nodes are bones, and which bone each is.
pub(crate) struct Joints(std::collections::HashMap<usize, usize>);

impl Joints {
    pub(crate) fn of(skins: &[gizmo_renderer::SkeletonHierarchy]) -> Self {
        Self(skins.first().map(|s| s.joints.iter().enumerate().map(|(bone, j)| (j.node_index, bone)).collect()).unwrap_or_default())
    }
}

/// Where a node sits in its model: the model's root, and the nearest bone above it with the
/// transform from that bone to the node's parent, if there is a bone above it.
#[derive(Clone, Copy)]
pub(crate) struct Place {
    pub(crate) root: Entity,
    pub(crate) bone: Option<(usize, gizmo_math::Mat4)>,
}

pub(crate) fn spawn_gltf_node_flat(
    world: &mut World,
    node: &gizmo_renderer::asset::GltfNodeData,
    parent_id: Entity,
    place: Place,
    joints: &Joints,
    default_mat: Material,
    attach_colliders: bool,
    skeletons: &[gizmo_renderer::components::Skeleton],
) {
    use gizmo_core::component::{Children, Parent};
    // A mesh hung on a bone without being skinned to it — a helmet on the head, a cape on the
    // chest, a sword in a hand — follows that bone's animated pose: the bone entities the file's
    // nodes become are not posed (the skeleton is), so parented to one it would stay where the
    // bind pose left it while the body moved. It hangs from the model's root instead, its place
    // written each frame by `BoneAttachmentSystem` from the bone's pose and its offset from it.
    let local = {
        let r = Quat::from_xyzw(node.rotation[0], node.rotation[1], node.rotation[2], node.rotation[3]);
        let r = if r.is_nan() || r.length() < 0.0001 { Quat::IDENTITY } else { r.normalize() };
        gizmo_math::Mat4::from_scale_rotation_translation(Vec3::from(node.scale), r, Vec3::from(node.translation))
    };
    let on_bone = place.bone.filter(|_| !node.primitives.is_empty() && node.skin_index.is_none());
    let parent_id = if on_bone.is_some() { place.root } else { parent_id };
    let entity = world.spawn();
    let name = node.name.clone().unwrap_or_else(|| "GLTF_Node".to_string());
    world.add_component(entity, EntityName(name));
    if let Some(parent) = world.live(parent_id) {
        world.add_component(entity, Parent(parent));
    }
    world.add_component(entity, Children(Vec::new()));

    {
        let mut ch_store = world.query_mut::<&mut Children>();
        // Safe to push since entity just spawned and didn't trigger any complex re-borrow updates
        if let Some(mut parent_ch) = ch_store.get_mut(parent_id) {
            parent_ch.0.push(entity);
        }
    }

    let raw_rot = Quat::from_xyzw(
        node.rotation[0],
        node.rotation[1],
        node.rotation[2],
        node.rotation[3],
    );
    let rot = if raw_rot.is_nan() || raw_rot.length() < 0.0001 {
        Quat::IDENTITY
    } else {
        raw_rot.normalize()
    };

    let mut t = Transform::new(Vec3::new(
        node.translation[0],
        node.translation[1],
        node.translation[2],
    ))
    .with_rotation(rot)
    .with_scale(Vec3::new(node.scale[0], node.scale[1], node.scale[2]));
    t.update_local_matrix();

    if node.skin_index.is_some() || node.name.as_deref() == Some("Armature") {
        t = Transform::default();
        t.update_local_matrix();
    }
    
    world.add_component(entity, t);
    world.add_component(entity, gizmo_physics_core::components::GlobalTransform::default());
    if let Some((bone, above)) = on_bone {
        world.add_component(
            entity,
            gizmo_renderer::components::BoneAttachment { bone_index: bone, offset: above * local },
        );
    }
    // What the node's children inherit: a bone of their own, or the way down from the one above.
    let below = match joints.0.get(&node.index) {
        Some(&bone) => Place { root: place.root, bone: Some((bone, gizmo_math::Mat4::IDENTITY)) },
        None => Place { root: place.root, bone: place.bone.map(|(b, above)| (b, above * local)) },
    };

    // Per-node/per-primitive → iç-döngü/özyineleme detayı, bu yüzden trace! (bir glTF yüzlerce
    // node içerebilir; aggregate özet çağıran `spawn_gltf`'in info! satırında verilir).
    tracing::trace!(
        entity = entity.id(),
        name = ?node.name,
        primitives = node.primitives.len(),
        children = node.children.len(),
        "glTF node spawn'lanıyor"
    );
    let mut newly_added_prims = Vec::new();
    for (mesh, mat_opt) in node.primitives.iter() {
        tracing::trace!(
            source = %mesh.source,
            bounds_min = ?mesh.bounds.min,
            bounds_max = ?mesh.bounds.max,
            "glTF primitive spawn'lanıyor"
        );
        let prim = world.spawn();
        let mut prim_t = Transform::new(Vec3::ZERO);
        prim_t.update_local_matrix();
        world.add_component(prim, prim_t);
        world.add_component(prim, gizmo_physics_core::components::GlobalTransform::default());
        world.add_component(prim, Parent(entity));
        world.add_component(prim, Children(Vec::new()));

        newly_added_prims.push(prim);

        world.add_component(prim, mesh.clone());
        // The mesh's cache key (`gltf_mesh_<file>_<node>_p<n>`) as its source, so a scene saves the
        // model and the loader reads the file again when it opens. Without it a saved model
        // reopened with no geometry at all.
        world.add_component(prim, gizmo_core::component::MeshSource(mesh.source.clone()));
        world.add_component(prim, mat_opt.clone().unwrap_or_else(|| default_mat.clone()));
        world.add_component(prim, MeshRenderer::new());

        if let Some(skin_idx) = node.skin_index {
            if skin_idx < skeletons.len() {
                world.add_component(prim, skeletons[skin_idx].clone());
            }
        }

        if attach_colliders {
            let extents = (mesh.bounds.max - mesh.bounds.min) / 2.0;
            let center_offset = (mesh.bounds.max + mesh.bounds.min) / 2.0;
            let cx = extents.x.max(0.01);
            let cy = extents.y.max(0.01);
            let cz = extents.z.max(0.01);
            
            // Eğer merkeze tam oturmuyorsa Compound(offset_box) kullanarak hizala
            if center_offset.length_squared() > 0.0001 {
                world.add_component(
                    prim,
                    gizmo_physics_core::Collider::offset_box(center_offset.into(), gizmo_math::Vec3::new(cx, cy, cz)),
                );
            } else {
                world.add_component(prim, gizmo_physics_core::Collider::new_aabb(cx, cy, cz));
            }
        }
    }

    // Pulling query_mut OUTSIDE the loop avoiding multiple overlapping mutable queries
    if !newly_added_prims.is_empty() {
        {
            let mut ch_store = world.query_mut::<&mut Children>();
            if let Some(mut parent_ch) = ch_store.get_mut(entity) {
                parent_ch.0.extend(newly_added_prims);
            }
        }
    }

    for child_node in &node.children {
        spawn_gltf_node_flat(
            world,
            child_node,
            entity,
            below,
            joints,
            default_mat.clone(),
            attach_colliders,
            skeletons,
        );
    }
}


/// Builds every glTF model asked for by name: a root carrying a
/// [`GltfModel`](gizmo_renderer::gltf_model::GltfModel) and nothing under it gets the file's nodes,
/// meshes and materials as its descendants, exactly as [`Commands::spawn_gltf`](crate::spawner::Commands::spawn_gltf) would have made
/// them, its own `Transform` kept. What follows in the same frame,
/// [`resolve_gltf_models`](gizmo_renderer::gltf_model::resolve_gltf_models), then gives it its
/// skeletons and its clips — to the [`AnimationPlayer`](gizmo_renderer::components::AnimationPlayer)
/// already on the root if the game put one there, so a clip chosen before the model was built is
/// the one it starts in.
///
/// This is how code that holds only the [`World`] — a game's update, a chunk being loaded — places
/// a model: the file is imported once per path and every later instance is built from the cache,
/// each with its own skeleton. A file that will not load is warned about and the root is left
/// empty (and marked, so it is not tried every frame).
pub(crate) fn build_requested_gltf_models(world: &mut World, renderer: &Renderer) {
    use gizmo_core::component::Children;
    use gizmo_renderer::gltf_model::{GltfModel, GltfModelResolved};
    let pending: Vec<(Entity, String)> = {
        let resolved = world.query::<&GltfModelResolved>();
        let children = world.query::<&Children>();
        world
            .query::<(Entity, &GltfModel)>()
            .iter()
            .filter(|(e, _)| resolved.get(*e).is_none() && children.get(*e).is_none_or(|c| c.0.is_empty()))
            .map(|(e, m)| (e, m.path.clone()))
            .collect()
    };
    if pending.is_empty() {
        return;
    }
    let (scenes, default_mat) = {
        let mut assets = renderer.asset_manager.write().unwrap_or_else(|p| p.into_inner());
        let white = assets.create_white_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout);
        let scenes: Vec<_> = pending
            .iter()
            .map(|(_, path)| assets.gltf_scene(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout, path))
            .collect();
        (scenes, Material::new(white))
    };
    for ((root, path), scene) in pending.into_iter().zip(scenes) {
        let scene = match scene {
            Ok(scene) => scene,
            Err(error) => {
                tracing::warn!(path, %error, "[GltfModel] the model's file would not load; nothing built");
                world.add_component(root, GltfModelResolved);
                continue;
            }
        };
        if world.get::<Children>(root).is_none() {
            world.add_component(root, Children(Vec::new()));
        }
        // No skeletons here: the resolver gives each skinned mesh its own, found by its source.
        let joints = Joints::of(&scene.skeletons);
        for node in &scene.roots {
            spawn_gltf_node_flat(world, node, root, Place { root, bone: None }, &joints, default_mat.clone(), false, &[]);
        }
        tracing::debug!(path, entity = root.id(), "[GltfModel] built from its file");
    }
    // Placed this frame: the new nodes' world matrices from their root's, before anything draws
    // them — otherwise a model's first frame is drawn at the origin.
    crate::systems::render::ensure_global_transforms(world);
}
