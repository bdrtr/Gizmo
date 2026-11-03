//! Turning a `Terrain` recipe into the mesh a draw path can actually draw.
//!
//! [`Terrain`](crate::renderer::components::Terrain) is a *recipe* — a heightmap path and the
//! extents to stretch it over — and the thing that gets drawn is the `Mesh` built from it. Until
//! this module existed, the only code in the workspace that performed that conversion lived in
//! `gizmo-studio`'s own render file, driven by an editor-only request queue that two places push
//! to: an inspector slider moving, and the ➕ menu adding the component. Both are edits.
//!
//! Nothing pushed a request when a scene **loaded**, and the engine's own frame did not know the
//! type at all. So the capability was authorable and then gone:
//!
//! - save a scene with a terrain, reopen it: the `Terrain` component came back (it is registered)
//!   and the `Mesh` did not, because `Mesh` owns GPU buffers and no file can hold one — leaving an
//!   entity that says it is a terrain and draws nothing.
//! - export the game: `PlayLoop::step` and `default_render_pass` are the exported frame, and
//!   neither named `Terrain`. A level built around a landscape shipped without it.
//!
//! The trigger here is **presence**, not a request: an entity that has the recipe and no mesh
//! wants one, which is true on load, on export and on the frame after a component is added, where
//! a queue of edits is true for none of those. The editor keeps its request path — regenerating
//! after a slider moves is a different question, and one this system cannot answer, because by
//! then the entity has a `Mesh` and looks finished.
//!
//! What it builds beside the mesh — a white material if the entity has none, a collider and a
//! static body — is what the editor builds too: "the export behaves like Play" is the contract,
//! and both now call [`terrain_collider`].
//!
//! **The collider was a box until 2026-11-04**, `width/2 × max_height/2 × depth/2` about the
//! entity — while the mesh runs from `0` to `max_height` — so every body on a terrain rested on a
//! flat lid at half its tallest hill: in the valleys it floated, on the peaks it sank in. It is
//! the heightfield of the same samples now, and the mesh is split along the heightfield's own
//! diagonal ([`AssetManager::terrain_triangles`](crate::renderer::asset::AssetManager::terrain_triangles)),
//! so what is drawn is what is hit.

use crate::renderer::components::{Mesh, MeshRenderer, Terrain};
use crate::renderer::Renderer;

/// The collider of a terrain whose heightmap sampled to `heights` (row-major, `0..=1`, `cols`
/// along X, `rows` along Z — what [`AssetManager::create_terrain`](crate::renderer::asset::AssetManager::create_terrain)
/// returns beside the mesh) and whose recipe is `width × depth × max_height`: a
/// [`Collider::heightfield`](crate::physics::Collider::heightfield) whose every sample sits on
/// the mesh vertex of the same row and column, centred on the entity like the mesh.
pub fn terrain_collider(heights: Vec<f32>, cols: u32, rows: u32, width: f32, depth: f32, max_height: f32) -> crate::physics::Collider {
    let cell = |extent: f32, n: u32| if n > 1 { extent / (n - 1) as f32 } else { extent };
    crate::physics::Collider::heightfield(
        heights,
        rows,
        cols,
        crate::math::Vec3::new(cell(width, cols), max_height, cell(depth, rows)),
    )
}

/// A terrain whose heightmap could not be read, so the retry stops.
///
/// Without it the system would re-open a missing or corrupt image **every frame** — the entity
/// keeps its recipe and never gains a mesh, which is exactly the condition that selects it. The
/// editor's request path is unaffected: it does its own conversion, so fixing the path in the
/// inspector still rebuilds.
#[derive(Clone, Copy)]
struct TerrainBuildFailed;

impl gizmo_core::component::Component for TerrainBuildFailed {
    fn storage_type() -> gizmo_core::component::StorageType {
        gizmo_core::component::StorageType::SparseSet
    }
}

/// Whether `collider` is the box every terrain was given before it had a heightfield — which a
/// scene saved then carries in its file, and which would otherwise survive the load, since a
/// terrain that already has a collider keeps it.
fn is_the_old_box(collider: &crate::physics::Collider, width: f32, depth: f32, max_height: f32) -> bool {
    matches!(&collider.shape, crate::physics::ColliderShape::Box(b)
        if b.half_extents == crate::math::Vec3::new(width / 2.0, max_height / 2.0, depth / 2.0))
}

/// Which entities want a mesh built this frame, and the recipe to build each from.
///
/// Split out from the system because **the trigger is the whole defect**: the conversion itself
/// has worked since the day the editor could ask for it, and what was missing was any answer to
/// "who wants one, and when". This half needs no GPU, so the answer is assertable.
///
/// Collecting before mutating is not a style choice either: `add_component` is a structural change
/// and cannot run while a query borrow is live — the same reason `ensure_global_transforms`
/// collects first.
fn recipes_awaiting_a_mesh(
    world: &crate::core::World,
) -> Vec<(u32, String, f32, f32, f32)> {
    let terrains = world.borrow::<Terrain>();
    let meshes = world.borrow::<Mesh>();
    let failed = world.borrow::<TerrainBuildFailed>();
    let mut pending = Vec::new();
    for (id, terrain) in terrains.iter() {
        // Already built, already failed, or never authored — an empty path is a `Terrain` a user
        // has added and not yet pointed at a heightmap, which is a normal state in the inspector
        // and not an error to log every frame.
        if meshes.contains(id) || failed.contains(id) || terrain.heightmap_path.is_empty() {
            continue;
        }
        pending.push((
            id,
            terrain.heightmap_path.clone(),
            terrain.width,
            terrain.depth,
            terrain.max_height,
        ));
    }
    pending
}

/// Builds the mesh for every entity carrying a [`Terrain`] recipe and no mesh yet.
///
/// Cheap on the frames where there is nothing to do: one query over entities that have a
/// `Terrain`, which in almost every scene is none or one.
pub fn terrain_mesh_system(world: &mut crate::core::World, renderer: &Renderer) {
    for (id, path, width, depth, max_height) in recipes_awaiting_a_mesh(world) {
        let built = crate::renderer::asset::AssetManager::create_terrain(
            &renderer.device,
            &path,
            width,
            depth,
            max_height,
        );
        let Some(entity) = world.get_entity(id) else {
            continue;
        };
        let (mesh, heights, cols, rows) = match built {
            Ok(built) => built,
            Err(e) => {
                tracing::warn!(
                    heightmap = %path,
                    error = %e,
                    "[Terrain] heightmap could not be read; this entity will not be retried"
                );
                world.add_component(entity, TerrainBuildFailed);
                continue;
            }
        };

        // A terrain with no material would be drawn with nothing to sample, so it gets the same
        // 1×1 white stand-in the editor gives it. Only when the entity has none: a material the
        // author (or a loaded scene) already chose must survive.
        if !world.borrow::<crate::prelude::Material>().contains(id) {
            let white = renderer.create_texture(&[255, 255, 255, 255], 1, 1);
            world.add_component(
                entity,
                crate::prelude::Material::new(std::sync::Arc::new(white)),
            );
        }
        if !world.borrow::<MeshRenderer>().contains(id) {
            world.add_component(entity, MeshRenderer::new());
        }
        let collider = world.borrow::<crate::physics::Collider>().get(id).cloned();
        let stand_in = collider.as_ref().is_some_and(|c| is_the_old_box(c, width, depth, max_height));
        if collider.is_none() || stand_in {
            world.add_component(entity, terrain_collider(heights, cols, rows, width, depth, max_height));
        }
        if collider.is_none() {
            // Static, or the landscape falls out of the world under gravity.
            world.add_component(entity, crate::physics::RigidBody::new_static());
        }
        world.add_component(entity, mesh);

        tracing::info!(heightmap = %path, width, depth, max_height, "[Terrain] mesh built");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::World;

    /// A 4 × 3 field with every cell sloped and one saddle, the case a wrong diagonal shows on.
    const HEIGHTS: [f32; 12] = [0.0, 0.5, 0.2, 0.9, 0.3, 1.0, 0.1, 0.4, 0.8, 0.2, 0.7, 0.0];
    const W: f32 = 30.0;
    const D: f32 = 20.0;
    const H: f32 = 6.0;

    fn field() -> gizmo_physics_core::components::HeightfieldShape {
        match terrain_collider(HEIGHTS.to_vec(), 4, 3, W, D, H).shape {
            crate::physics::ColliderShape::Heightfield(hf) => hf,
            other => panic!("a terrain collides as a heightfield, not {other:?}"),
        }
    }

    /// Every triangle the mesh draws is one of the triangles the collider resolves against.
    #[test]
    fn the_terrain_collider_is_the_surface_the_mesh_draws() {
        let hf = field();
        let drawn = crate::renderer::asset::AssetManager::terrain_triangles(&HEIGHTS, 4, 3, W, D, H);
        assert_eq!(drawn.len(), 3 * 2 * 6, "two triangles for each of the 3 × 2 cells");
        let hit: Vec<[gizmo_math::Vec3; 3]> =
            (0..2u32).flat_map(|r| (0..3u32).map(move |c| (r, c))).flat_map(|(r, c)| hf.cell_triangles(r, c)).collect();
        let close = |a: gizmo_math::Vec3, b: gizmo_math::Vec3| (a - b).length() < 1e-4;
        for tri in drawn.chunks(3) {
            let p: Vec<gizmo_math::Vec3> = tri.iter().map(|v| gizmo_math::Vec3::from_array(v.position)).collect();
            assert!(
                hit.iter().any(|t| p.iter().all(|q| t.iter().any(|r| close(*q, *r)))),
                "the mesh draws {p:?}, which the collider does not have"
            );
        }
    }

    /// A ray straight down lands at the height the mesh is drawn at — inside cells, not only on
    /// the samples, which is where a box or the other diagonal is wrong.
    #[test]
    fn a_ray_down_lands_on_the_drawn_surface() {
        use gizmo_physics_core::raycast::{Ray, Raycast};
        let collider = terrain_collider(HEIGHTS.to_vec(), 4, 3, W, D, H);
        let drawn = crate::renderer::asset::AssetManager::terrain_triangles(&HEIGHTS, 4, 3, W, D, H);
        for tri in drawn.chunks(3) {
            let p: Vec<gizmo_math::Vec3> = tri.iter().map(|v| gizmo_math::Vec3::from_array(v.position)).collect();
            // A point well inside the triangle, and the mesh's height there.
            let inside = p[0] * 0.5 + p[1] * 0.3 + p[2] * 0.2;
            let from = gizmo_math::Vec3::new(inside.x, 100.0, inside.z);
            let (d, _) = Raycast::ray_shape(&Ray::new(from, -gizmo_math::Vec3::Y), &collider.shape, &crate::physics::Transform::default())
                .expect("the ray misses the terrain");
            let y = 100.0 - d;
            assert!((y - inside.y).abs() < 1e-3, "drawn at {:.3}, collided at {y:.3}", inside.y);
        }
    }

    /// A scene saved while terrains were given a box brings that box back; it is the stand-in,
    /// recognised as such, and nothing else is.
    #[test]
    fn the_old_box_is_recognised_and_nothing_else() {
        let old = crate::physics::Collider::box_collider(gizmo_math::Vec3::new(W / 2.0, H / 2.0, D / 2.0));
        assert!(is_the_old_box(&old, W, D, H));
        let authored = crate::physics::Collider::box_collider(gizmo_math::Vec3::new(W / 2.0, 1.0, D / 2.0));
        assert!(!is_the_old_box(&authored, W, D, H), "a box the author chose stays");
        assert!(!is_the_old_box(&terrain_collider(HEIGHTS.to_vec(), 4, 3, W, D, H), W, D, H));
    }

    fn a_world_with_a_terrain(path: &str) -> (World, u32) {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(
            e,
            Terrain::new(path.to_string(), 100.0, 80.0, 20.0),
        );
        (world, e.id())
    }

    /// **The case that was never asked.** A scene that has just loaded is exactly this: the recipe
    /// is back (it is registered and serialises), and the mesh cannot be — `Mesh` owns GPU buffers.
    /// Nothing pushed an editor request, so before this system that entity waited for ever.
    #[test]
    fn a_recipe_with_no_mesh_is_waiting_for_one() {
        let (world, id) = a_world_with_a_terrain("heightmaps/valley.png");
        let pending = recipes_awaiting_a_mesh(&world);
        assert_eq!(pending.len(), 1, "the loaded terrain wants a mesh");
        assert_eq!(pending[0].0, id);
        assert_eq!(pending[0].1, "heightmaps/valley.png");
        assert_eq!(
            (pending[0].2, pending[0].3, pending[0].4),
            (100.0, 80.0, 20.0),
            "the extents come from the recipe, not from a default"
        );
    }

    /// A `Terrain` added from the ➕ menu and not yet pointed at a file is a normal inspector
    /// state, not a failure — selecting it would mean opening `""` on every frame and logging the
    /// same error for as long as the user takes to fill the field in.
    #[test]
    fn a_recipe_with_no_heightmap_yet_is_not_an_error_to_retry() {
        let (world, _) = a_world_with_a_terrain("");
        assert!(recipes_awaiting_a_mesh(&world).is_empty());
    }

    /// And a heightmap that could not be read is tried **once**. Without the marker the entity
    /// still has a recipe and still has no mesh — the very condition that selects it — so a
    /// missing file would re-open at the frame rate for the life of the process.
    #[test]
    fn a_heightmap_that_could_not_be_read_is_not_tried_again() {
        let (mut world, id) = a_world_with_a_terrain("heightmaps/does_not_exist.png");
        assert_eq!(recipes_awaiting_a_mesh(&world).len(), 1, "the first frame tries");

        let entity = world.get_entity(id).expect("the entity is alive");
        world.add_component(entity, TerrainBuildFailed);
        assert!(
            recipes_awaiting_a_mesh(&world).is_empty(),
            "and no frame after it does"
        );
    }

    /// An entity with no recipe is not a terrain, whatever else it carries.
    #[test]
    fn an_entity_without_a_recipe_is_never_selected() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, MeshRenderer::new());
        assert!(recipes_awaiting_a_mesh(&world).is_empty());
    }
}
