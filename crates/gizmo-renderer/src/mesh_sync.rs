//! A mesh built in code, described so a scene can keep it.
//!
//! A scene saves an entity's geometry as its `MeshSource` key and nothing else, and the spawner
//! helpers, `SimpleApp`'s shapes and a game's own `AssetManager::create_sphere(..)` hand an entity a
//! GPU `Mesh` with no key beside it: such an entity saved as a transform with nothing to draw. Every
//! mesh says what it is in [`Mesh::source`] — `sphere_0.5_20_20`, `obj:rock.obj`, a glTF sub-mesh —
//! and when that is a key the engine can build again ([`AssetManager::rebuilds_mesh`]),
//! [`describe_meshes`] writes it as the entity's `MeshSource`.
//!
//! A mesh made from points in code (an extrusion, a `MeshData`) has no such key and is left
//! undescribed, as before.

use gizmo_core::component::MeshSource;
use gizmo_core::{Entity, World};

use crate::asset::AssetManager;
use crate::components::Mesh;

/// The [`Mesh::source`] an entity's description was last brought in line with.
#[derive(Clone)]
struct DescribedMesh(String);
impl gizmo_core::component::Component for DescribedMesh {}

/// What [`describe_meshes`] does for one entity.
#[derive(Debug, PartialEq)]
enum Describe {
    /// Nothing changed.
    Nothing,
    /// Remember this mesh; the `MeshSource` there already is stays (a scene's key, or a game's).
    Adopt,
    /// Write this key as the `MeshSource`.
    Write,
    /// The mesh is one no key builds: take a `MeshSource` that names the old one away.
    Clear,
}

fn decide(source: &str, has_source: bool, described: Option<&str>) -> Describe {
    match described {
        Some(d) if d == source => Describe::Nothing,
        // Seen for the first time: a key that is already there was given on purpose.
        None if has_source => Describe::Adopt,
        // New, or the mesh was swapped since: the description follows the mesh.
        _ if AssetManager::rebuilds_mesh(source) => Describe::Write,
        None => Describe::Adopt,
        Some(_) if has_source => Describe::Clear,
        Some(_) => Describe::Adopt,
    }
}

/// Gives every entity whose `Mesh` can be built again from its [`source`](Mesh::source) a
/// `MeshSource` naming it, and keeps it naming the mesh when the mesh is swapped — see the
/// [module docs](self). An entity that already had a `MeshSource` when first seen keeps it until its
/// mesh changes. Needs no GPU.
pub fn describe_meshes(world: &mut World) {
    let work: Vec<(Entity, String, Describe)> = {
        let sources = world.query::<&MeshSource>();
        let described = world.query::<&DescribedMesh>();
        world
            .query::<(Entity, &Mesh)>()
            .iter()
            .filter_map(|(e, mesh)| {
                let d = decide(&mesh.source, sources.get(e).is_some(), described.get(e).map(|d| d.0.as_str()));
                (d != Describe::Nothing).then(|| (e, mesh.source.clone(), d))
            })
            .collect()
    };
    for (entity, source, describe) in work {
        match describe {
            Describe::Write => world.add_component(entity, MeshSource(source.clone())),
            Describe::Clear => world.remove_component::<MeshSource>(entity),
            Describe::Adopt | Describe::Nothing => {}
        }
        world.add_component(entity, DescribedMesh(source));
    }
}

#[cfg(test)]
mod tests {
    use super::{decide, Describe};
    use crate::asset::AssetManager;

    /// The keys the builders name their meshes with are the keys the loader builds again, and the
    /// ones it cannot are not claimed.
    #[test]
    fn a_built_mesh_is_described_by_its_own_name() {
        for key in [
            "sphere_0.5_20_20", "plane_10", "plane_4_8", "circle_1.5_32", "cylinder_0.25_2_24", "cone_1_2_12",
            "torus_1_0.25_24_12", "capsule_0.5_1_8_16", "sprite_quad_2_1", "tetrahedron_1", "conical_frustum_1_0.5_2_16",
            "standard_cube", "sphere", "obj:models/rock.obj", "gltf_mesh_assets/car.glb_Body_p0",
        ] {
            assert!(AssetManager::rebuilds_mesh(key), "{key} is built again and was not recognised");
        }
        for key in ["extrusion_6", "sweep_4_12", "editor_grid", "gizmo_arrow", "terrain:hm.png", "sphere_0.5_20", "sphere_x_1_1", "sphere_1_0_8", "cylinder_1_2_nan"] {
            assert!(!AssetManager::rebuilds_mesh(key), "{key} is not a key anything builds from");
        }
    }

    /// What happens to an entity's description, case by case.
    #[test]
    fn the_description_follows_the_mesh() {
        // New, with a buildable key: written. With one nothing builds: left alone.
        assert_eq!(decide("sphere_1_16_16", false, None), Describe::Write);
        assert_eq!(decide("extrusion_6", false, None), Describe::Adopt);
        // A key already there when first seen (a scene's, a game's) stays.
        assert_eq!(decide("sphere_1_16_16", true, None), Describe::Adopt);
        // Unchanged: nothing.
        assert_eq!(decide("sphere_1_16_16", true, Some("sphere_1_16_16")), Describe::Nothing);
        // Swapped for another buildable mesh: the key follows; for an unbuildable one: it goes.
        assert_eq!(decide("cone_1_2_12", true, Some("sphere_1_16_16")), Describe::Write);
        assert_eq!(decide("extrusion_6", true, Some("sphere_1_16_16")), Describe::Clear);
        assert_eq!(decide("extrusion_6", false, Some("sphere_1_16_16")), Describe::Adopt);
    }
}
