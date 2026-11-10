//! [`MeshData`] — geometry as data, uploaded by the draw path.

use std::sync::Arc;

use gizmo_math::Vec2;

use crate::asset::AssetManager;
use crate::gpu_types::Vertex;

/// A mesh that is still **data**: a flat triangle list the asset loader uploads the first frame
/// its entity has no [`Mesh`](crate::components::Mesh).
///
/// [`MeshSource`](gizmo_core::component::MeshSource) covers the built-in primitives and files;
/// anything else — an extrusion, a lathed vase, a terrain chunk a system just generated — needed
/// the `Device`, which an `on_enter` hook or a scheduled system does not have. This is the same
/// deferral for geometry made in code: build it anywhere, spawn it beside a material description
/// and a `MeshRenderer`, and it is drawn the next frame.
///
/// ```
/// use gizmo_math::Vec2;
/// use gizmo_renderer::components::MeshData;
///
/// let star: Vec<Vec2> = (0..10)
///     .map(|i| {
///         let a = i as f32 * std::f32::consts::TAU / 10.0;
///         let r = if i % 2 == 0 { 1.0 } else { 0.45 };
///         Vec2::new(a.cos() * r, a.sin() * r)
///     })
///     .collect();
/// let data = MeshData::extrude(&star, 0.3).expect("a star triangulates");
/// assert_eq!(data.vertices().len() % 3, 0);
/// ```
///
/// **Clones share one upload.** The vertices sit behind an `Arc`, and the loader keys its upload
/// on that allocation: a hundred entities spawned with clones of one `MeshData` draw one vertex
/// buffer, while two `MeshData` built separately are two uploads even if their vertices are equal
/// (comparing contents would cost more than the upload it saved).
///
/// Not saved with a scene — a scene stores `MeshSource` keys, and geometry made in code is the
/// game's to make again.
#[derive(Clone, Debug)]
pub struct MeshData {
    vertices: Arc<[Vertex]>,
}

impl MeshData {
    /// A mesh from a flat, counter-clockwise triangle list. Empty input is accepted and uploads
    /// nothing.
    #[must_use]
    pub fn new(vertices: impl Into<Arc<[Vertex]>>) -> Self {
        Self { vertices: vertices.into() }
    }

    /// The triangle list.
    #[must_use]
    pub fn vertices(&self) -> &[Vertex] {
        &self.vertices
    }

    /// The address the loader keys its shared upload on — the same for every clone. An address is a
    /// key only while its allocation lives, so whatever keeps one keeps a clone beside it: the
    /// upload cache does, and so does the loader's record of what an entity was uploaded from.
    pub(crate) fn key(&self) -> usize {
        Arc::as_ptr(&self.vertices) as *const Vertex as usize
    }

    /// Whether `other` is a clone of this one (the same allocation), not merely equal.
    pub(crate) fn same_allocation(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.vertices, &other.vertices)
    }

    /// A closed 2-D outline extruded along Z by `depth`, centred — see
    /// [`extrude_data`](crate::asset::primitives::extrude::extrude_data). `None` when the outline
    /// cannot be triangulated.
    #[must_use]
    pub fn extrude(outline: &[Vec2], depth: f32) -> Option<Self> {
        crate::asset::primitives::extrude::extrude_data(outline, depth).map(Self::new)
    }

    /// A closed 2-D profile swept round the Y axis — see
    /// [`sweep_data`](crate::asset::primitives::extrude::sweep_data).
    #[must_use]
    pub fn sweep(profile: &[Vec2], radius: f32, segments: u32) -> Option<Self> {
        crate::asset::primitives::extrude::sweep_data(profile, radius, segments).map(Self::new)
    }

    /// A UV sphere of `radius`.
    #[must_use]
    pub fn sphere(radius: f32, stacks: u32, slices: u32) -> Self {
        Self::new(AssetManager::sphere_data(radius, stacks, slices))
    }

    /// A capped cylinder about Y, `height` tall, centred.
    #[must_use]
    pub fn cylinder(radius: f32, height: f32, segments: u32) -> Self {
        Self::new(AssetManager::cylinder_data(radius, height, segments))
    }

    /// A cone about Y with a capped base, `height` tall, centred, apex up.
    #[must_use]
    pub fn cone(radius: f32, height: f32, segments: u32) -> Self {
        Self::new(AssetManager::cone_data(radius, height, segments))
    }

    /// A capsule about Y: a cylinder of length `depth` with a hemisphere of `radius` on each end.
    #[must_use]
    pub fn capsule(radius: f32, depth: f32, latitudes: u32, longitudes: u32) -> Self {
        Self::new(AssetManager::capsule_data(radius, depth, latitudes, longitudes))
    }
}

gizmo_core::impl_component!(MeshData);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_share_an_allocation_and_separate_builds_do_not() {
        let a = MeshData::cylinder(1.0, 2.0, 12);
        let b = a.clone();
        let c = MeshData::cylinder(1.0, 2.0, 12);
        assert!(a.same_allocation(&b) && a.key() == b.key());
        assert!(!a.same_allocation(&c), "two builds are two meshes, even when equal");
        assert_eq!(a.vertices().len(), c.vertices().len());
    }

    #[test]
    fn every_builder_makes_a_triangle_list() {
        let outline = [Vec2::new(-1.0, -1.0), Vec2::new(1.0, -1.0), Vec2::new(0.0, 1.0)];
        for (name, m) in [
            ("sphere", MeshData::sphere(1.0, 8, 12)),
            ("cylinder", MeshData::cylinder(1.0, 2.0, 12)),
            ("cone", MeshData::cone(1.0, 2.0, 12)),
            ("capsule", MeshData::capsule(0.5, 1.0, 8, 12)),
            ("extrude", MeshData::extrude(&outline, 0.5).unwrap()),
            ("sweep", MeshData::sweep(&outline, 2.0, 12).unwrap()),
        ] {
            assert!(!m.vertices().is_empty() && m.vertices().len() % 3 == 0, "{name}: {}", m.vertices().len());
        }
        assert!(MeshData::extrude(&outline[..2], 0.5).is_none(), "a two-point outline is not a shape");
    }
}
