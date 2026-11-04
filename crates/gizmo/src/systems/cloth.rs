//! Drawing a [`Cloth`](gizmo_physics_soft::cloth::Cloth) component.
//!
//! A `Cloth` in the world was simulated — the physics step runs `cloth_step_system` over every
//! one — and never drawn: no draw path knew the type, and the cloth did not even keep the grid
//! width that turns its nodes back into a surface. `cloth_demo` kept its cloth out of the ECS and
//! rebuilt a GPU mesh itself in a render hook. Both draw paths call [`cloth_mesh_system`] now.

use crate::renderer::components::{MaterialDesc, MeshData, MeshRenderer};
use crate::renderer::Vertex;
use gizmo_math::Vec3;
use gizmo_physics_core::components::{GlobalTransform, Transform};
use gizmo_physics_soft::cloth::Cloth;

/// Gives every entity with a [`Cloth`] the mesh its nodes make this frame: a [`MeshData`] of the
/// cloth's whole cells ([`Cloth::surface_triangles`] — a tear is a hole), **both faces**, with
/// flat normals; and, the first time, a renderer, a `GlobalTransform` and — if it has none — a
/// plain white material.
///
/// The nodes are in world space; the mesh is written in the entity's own frame (through the
/// inverse of its `Transform`, if it has one), so a cloth entity can sit anywhere. Rewriting the
/// `MeshData` every frame re-uploads it, which is what a moving cloth costs.
pub fn cloth_mesh_system(world: &mut crate::core::World) {
    let built: Vec<(u32, MeshData)> = {
        let cloths = world.borrow::<Cloth>();
        let transforms = world.borrow::<Transform>();
        cloths
            .iter()
            .map(|(id, cloth)| {
                let to_local = transforms.get(id).map(|t| gizmo_math::Mat4::from_scale_rotation_translation(t.scale, t.rotation, t.position).inverse()).unwrap_or(gizmo_math::Mat4::IDENTITY);
                (id, MeshData::new(cloth_vertices(cloth, to_local)))
            })
            .collect()
    };
    for (id, mesh) in built {
        let Some(e) = world.get_entity(id) else { continue };
        let first = world.borrow::<MeshRenderer>().get(id).is_none();
        world.add_component(e, mesh);
        if first {
            world.add_component(e, MeshRenderer::new());
            if world.borrow::<GlobalTransform>().get(id).is_none() {
                world.add_component(e, GlobalTransform::default());
            }
            if world.borrow::<MaterialDesc>().get(id).is_none() {
                world.add_component(e, MaterialDesc::new().with_pbr(gizmo_math::Vec4::ONE, 0.8, 0.0));
            }
        }
    }
}

/// Both faces of every whole cell, flat-shaded, in the frame `to_local` maps into.
fn cloth_vertices(cloth: &Cloth, to_local: gizmo_math::Mat4) -> Vec<Vertex> {
    let tris = cloth.surface_triangles();
    let mut out = Vec::with_capacity(tris.len() * 6);
    for [a, b, c] in tris {
        let p = [a, b, c].map(|i| to_local.transform_point3(cloth.nodes[i].position));
        let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or(Vec3::Y);
        for (order, normal) in [([0, 1, 2], n), ([0, 2, 1], -n)] {
            for i in order {
                out.push(Vertex { position: p[i].to_array(), normal: normal.to_array(), ..Default::default() });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::World;

    /// A cloth entity gets a mesh of both faces of its whole cells, where its nodes are, and
    /// follows them.
    #[test]
    fn a_cloth_is_drawn_where_its_nodes_are() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::new(10.0, 0.0, 0.0)));
        world.add_component(e, Cloth::new(4, 3, 0.5, 1.0));
        cloth_mesh_system(&mut world);
        let verts: Vec<Vec3> = world.borrow::<MeshData>().get(e.id()).unwrap().vertices().iter().map(|v| Vec3::from_array(v.position)).collect();
        assert_eq!(verts.len(), 12 * 3 * 2, "two faces of two triangles of six cells");
        // Node (0, 0) is at the world origin; the entity sits at x = 10, so locally it is at −10.
        assert!(verts.iter().any(|v| (*v - Vec3::new(-10.0, 0.0, 0.0)).length() < 1e-5));
        assert!(world.borrow::<MeshRenderer>().get(e.id()).is_some());
        assert!(world.borrow::<MaterialDesc>().get(e.id()).is_some());

        world.borrow_mut::<Cloth>().get_mut(e.id()).unwrap().nodes[0].position = Vec3::new(10.0, -3.0, 0.0);
        cloth_mesh_system(&mut world);
        let moved = world.borrow::<MeshData>().get(e.id()).unwrap().vertices().iter().any(|v| (Vec3::from_array(v.position) - Vec3::new(0.0, -3.0, 0.0)).length() < 1e-5);
        assert!(moved, "the mesh did not follow the node");
    }

    /// A torn cloth draws the hole.
    #[test]
    fn a_tear_is_not_drawn() {
        let mut world = World::new();
        let e = world.spawn();
        let mut cloth = Cloth::new(4, 3, 0.5, 1.0);
        cloth.constraints.retain(|k| !((k.node_a == 5 && k.node_b == 1) || (k.node_a == 1 && k.node_b == 5)));
        world.add_component(e, cloth);
        cloth_mesh_system(&mut world);
        assert_eq!(world.borrow::<MeshData>().get(e.id()).unwrap().vertices().len(), 8 * 3 * 2);
    }
}
