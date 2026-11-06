//! Drawing a [`Cloth`](gizmo_physics_soft::cloth::Cloth) component — and a
//! [`Rope`](gizmo_physics_soft::rope::Rope), which had the same gap.
//!
//! A [`SoftBodyMesh`](gizmo_physics_soft::SoftBodyMesh) had it too, and is drawn by
//! [`soft_body_mesh_system`].
//!
//! A `Cloth` in the world was simulated — the physics step runs `cloth_step_system` over every
//! one — and never drawn: no draw path knew the type, and the cloth did not even keep the grid
//! width that turns its nodes back into a surface. `cloth_demo` kept its cloth out of the ECS and
//! rebuilt a GPU mesh itself in a render hook. Both draw paths call [`cloth_mesh_system`] now.

use gizmo_core::Entity;
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
        let transforms = world.borrow::<Transform>();
        world.query::<(Entity, &Cloth)>().unwrap().iter()
            .map(|(entity, cloth)| {
                let to_local = transforms.get(entity).map(|t| gizmo_math::Mat4::from_scale_rotation_translation(t.scale, t.rotation, t.position).inverse()).unwrap_or(gizmo_math::Mat4::IDENTITY);
                (entity.id(), MeshData::new(cloth_vertices(cloth, to_local)))
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

/// Gives every entity with a [`Rope`](gizmo_physics_soft::rope::Rope) the mesh its nodes make
/// this frame: a square tube [`Rope::thickness`](gizmo_physics_soft::rope::Rope::thickness)
/// across along every link, in the entity's own frame, plus — the first time — a renderer, a
/// `GlobalTransform` and a dark material if it has none.
///
/// A `Rope` in the world was stepped by the physics step and drawn by nothing, like `Cloth`;
/// `advanced_physics` kept its rope outside the ECS and placed a sphere on every node by hand.
pub fn rope_mesh_system(world: &mut crate::core::World) {
    use gizmo_physics_soft::rope::Rope;
    let built: Vec<(u32, MeshData)> = {
        let transforms = world.borrow::<Transform>();
        world.query::<(Entity, &Rope)>().unwrap().iter()
            .map(|(entity, rope)| {
                let to_local = transforms
                    .get(entity)
                    .map(|t| gizmo_math::Mat4::from_scale_rotation_translation(t.scale, t.rotation, t.position).inverse())
                    .unwrap_or(gizmo_math::Mat4::IDENTITY);
                (entity.id(), MeshData::new(rope_vertices(rope, to_local)))
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
                world.add_component(e, MaterialDesc::new().with_pbr(gizmo_math::Vec4::new(0.35, 0.27, 0.18, 1.0), 0.9, 0.0));
            }
        }
    }
}

/// Four flat sides along every link of the rope.
fn rope_vertices(rope: &gizmo_physics_soft::rope::Rope, to_local: gizmo_math::Mat4) -> Vec<Vertex> {
    let r = rope.thickness * 0.5;
    let mut out = Vec::with_capacity(rope.nodes.len().saturating_sub(1) * 24);
    for pair in rope.nodes.windows(2) {
        let (a, b) = (to_local.transform_point3(pair[0].position), to_local.transform_point3(pair[1].position));
        let along = (b - a).normalize_or(Vec3::Y);
        let side = along.any_orthonormal_vector();
        let up = along.cross(side);
        // The four faces, each a quad from `a` to `b` facing outward.
        for (n, t) in [(side, up), (up, -side), (-side, -up), (-up, side)] {
            let (c0, c1) = (n * r - t * r, n * r + t * r);
            let quad = [a + c0, b + c0, b + c1, a + c1];
            for i in [0, 1, 2, 0, 2, 3] {
                out.push(Vertex { position: quad[i].to_array(), normal: n.to_array(), ..Default::default() });
            }
        }
    }
    out
}

/// Gives every entity with a [`SoftBodyMesh`](gizmo_physics_soft::SoftBodyMesh) the mesh of its
/// surface this frame — the faces of its tetrahedra that belong to one tetrahedron only
/// ([`surface_faces`](gizmo_physics_soft::soft_contact::surface_faces)), outward, with normals
/// smoothed over the nodes so a wobbling jelly reads as one surface — in the entity's own frame;
/// and, the first time, a renderer, a `GlobalTransform` and a material if it has none.
///
/// A soft body in the world was stepped by the physics step and drawn as nothing but debug
/// lines (`physics_debug_system`), so a game had none to show.
pub fn soft_body_mesh_system(world: &mut crate::core::World) {
    use gizmo_physics_soft::SoftBodyMesh;
    let built: Vec<(u32, MeshData)> = {
        let transforms = world.borrow::<Transform>();
        world.query::<(Entity, &SoftBodyMesh)>().unwrap().iter()
            .map(|(entity, body)| {
                let to_local = transforms
                    .get(entity)
                    .map(|t| gizmo_math::Mat4::from_scale_rotation_translation(t.scale, t.rotation, t.position).inverse())
                    .unwrap_or(gizmo_math::Mat4::IDENTITY);
                (entity.id(), MeshData::new(soft_body_vertices(body, to_local)))
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
                world.add_component(e, MaterialDesc::new().with_pbr(gizmo_math::Vec4::new(0.9, 0.3, 0.4, 1.0), 0.3, 0.0));
            }
        }
    }
}

/// The outward surface triangles, normals averaged per node.
fn soft_body_vertices(body: &gizmo_physics_soft::SoftBodyMesh, to_local: gizmo_math::Mat4) -> Vec<Vertex> {
    let faces = gizmo_physics_soft::soft_contact::surface_faces(body);
    let local: Vec<Vec3> = body.nodes.iter().map(|n| to_local.transform_point3(n.position)).collect();
    let mut normals = vec![Vec3::ZERO; local.len()];
    for f in &faces {
        let [a, b, c] = f.map(|i| local[i as usize]);
        let n = (b - a).cross(c - a); // area-weighted
        for &i in f {
            normals[i as usize] += n;
        }
    }
    let mut out = Vec::with_capacity(faces.len() * 3);
    for f in &faces {
        for &i in f {
            let i = i as usize;
            out.push(Vertex { position: local[i].to_array(), normal: normals[i].normalize_or(Vec3::Y).to_array(), ..Default::default() });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::World;
    use gizmo_math::Vec2;

    /// A cloth entity gets a mesh of both faces of its whole cells, where its nodes are, and
    /// follows them.
    #[test]
    fn a_cloth_is_drawn_where_its_nodes_are() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::new(10.0, 0.0, 0.0)));
        world.add_component(e, Cloth::new(4, 3, 0.5, 1.0));
        cloth_mesh_system(&mut world);
        let verts: Vec<Vec3> = world.borrow::<MeshData>().get(e).unwrap().vertices().iter().map(|v| Vec3::from_array(v.position)).collect();
        assert_eq!(verts.len(), 12 * 3 * 2, "two faces of two triangles of six cells");
        // Node (0, 0) is at the world origin; the entity sits at x = 10, so locally it is at −10.
        assert!(verts.iter().any(|v| (*v - Vec3::new(-10.0, 0.0, 0.0)).length() < 1e-5));
        assert!(world.borrow::<MeshRenderer>().get(e).is_some());
        assert!(world.borrow::<MaterialDesc>().get(e).is_some());

        world.borrow_mut::<Cloth>().get_mut(e).unwrap().nodes[0].position = Vec3::new(10.0, -3.0, 0.0);
        cloth_mesh_system(&mut world);
        let moved = world.borrow::<MeshData>().get(e).unwrap().vertices().iter().any(|v| (Vec3::from_array(v.position) - Vec3::new(0.0, -3.0, 0.0)).length() < 1e-5);
        assert!(moved, "the mesh did not follow the node");
    }

    /// A rope entity gets a tube along its links, where its nodes are.
    #[test]
    fn a_rope_is_drawn_along_its_links() {
        use gizmo_physics_soft::rope::Rope;
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Rope::new(Vec3::new(0.0, 5.0, 0.0), Vec3::NEG_Y, 4, 0.5, 0.1, true, false));
        rope_mesh_system(&mut world);
        let verts: Vec<Vec3> = world.borrow::<MeshData>().get(e).unwrap().vertices().iter().map(|v| Vec3::from_array(v.position)).collect();
        assert_eq!(verts.len(), 4 * 4 * 6, "four links, four sides, two triangles each");
        // Every vertex lies within the rope's half-thickness (diagonally) of the x = z = 0 line.
        assert!(verts.iter().all(|v| Vec2::new(v.x, v.z).length() < 0.04));
        assert!(verts.iter().any(|v| (v.y - 3.0).abs() < 1e-4), "the tube reaches the last node at y = 3");
        assert!(world.borrow::<MeshRenderer>().get(e).is_some());
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
        assert_eq!(world.borrow::<MeshData>().get(e).unwrap().vertices().len(), 8 * 3 * 2);
    }

    /// A soft body entity gets a mesh of its outer surface only, facing out, where its nodes are.
    #[test]
    fn a_soft_body_is_drawn_as_its_surface() {
        use gizmo_physics_soft::SoftBodyMesh;
        let mut world = World::new();
        let e = world.spawn();
        let body = SoftBodyMesh::block(Vec3::new(4.0, 0.0, 0.0), Vec3::ONE, [2, 2, 2], 8.0, 1000.0, 0.3).unwrap();
        world.add_component(e, body);
        soft_body_mesh_system(&mut world);
        let verts: Vec<(Vec3, Vec3)> = world
            .borrow::<MeshData>()
            .get(e)
            .unwrap()
            .vertices()
            .iter()
            .map(|v| (Vec3::from_array(v.position), Vec3::from_array(v.normal)))
            .collect();
        assert_eq!(verts.len(), 6 * 4 * 2 * 3, "six sides of four squares, two triangles each — no inner faces");
        let centre = Vec3::new(4.5, 0.5, 0.5);
        for tri in verts.chunks(3) {
            let (a, b, c) = (tri[0].0, tri[1].0, tri[2].0);
            assert!((b - a).cross(c - a).dot(a - centre) > 0.0, "a triangle faces into the body");
        }
        assert!(verts.iter().all(|(p, n)| n.dot(*p - centre) > 0.0), "a normal points inward");
        assert!(world.borrow::<MeshRenderer>().get(e).is_some());
    }
}
