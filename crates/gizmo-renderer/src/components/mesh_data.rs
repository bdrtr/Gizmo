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
/// **Saved with a scene**, vertices and all (decided 2026-11-14), and so carried through the
/// editor's Play/Stop snapshot too: an entity a game made from points and the game then destroyed
/// comes back with its geometry, and a level baked in code saves as what it is. A scene stores one
/// list per vertex channel and leaves out a channel every vertex holds at its default — see
/// `MeshDataFile` — so a plain extrusion is its positions, normals and UVs. What that costs the
/// editor, measured 2026-11-14 in release: a 59 400-vertex sphere takes 29 ms to capture into the
/// ▶ snapshot and 36 ms to restore at ⏹, about half a millisecond per thousand vertices.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(into = "MeshDataFile", try_from = "MeshDataFile")]
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

    /// The same mesh with a second UV set laid out for a lightmap ([`crate::lightmap_uv::unwrap`]):
    /// what a mesh made in code needs before it can be baked and drawn lightmapped.
    pub fn with_lightmap_uvs(&self, settings: &crate::lightmap_uv::UnwrapSettings) -> Result<Self, crate::lightmap_uv::UnwrapError> {
        let mut vertices = self.vertices.to_vec();
        crate::lightmap_uv::unwrap(&mut vertices, settings)?;
        Ok(Self::new(vertices))
    }

    /// Whether any vertex has a second UV set (`Vertex::lightmap_uv` not zero).
    #[must_use]
    pub fn has_lightmap_uvs(&self) -> bool {
        self.vertices.iter().any(|v| v.lightmap_uv != [0.0, 0.0])
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

    /// A **convex** 2-D outline extruded along Y by `depth` — see
    /// [`AssetManager::create_convex_extrusion`].
    #[must_use]
    pub fn convex_extrusion(points: &[[f32; 2]], depth: f32) -> Self {
        Self::new(AssetManager::convex_extrusion_data(points, depth))
    }

    /// The ring between an inner and an outer 2-D outline, extruded along Y by `depth` — see
    /// [`AssetManager::create_ring_extrusion`].
    #[must_use]
    pub fn ring_extrusion(inner: &[[f32; 2]], outer: &[[f32; 2]], depth: f32) -> Self {
        Self::new(AssetManager::ring_extrusion_data(inner, outer, depth))
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

/// How a [`MeshData`] is written in a scene: one list per vertex channel, positions always, every
/// other channel left out when all of its vertices hold [`Vertex::default`]'s value there (no UVs,
/// white, no skin, the default tangent). Read back, a channel left out is that default, and one
/// whose length is not the positions' is an error rather than a mesh with holes in it.
#[derive(serde::Serialize, serde::Deserialize)]
struct MeshDataFile {
    positions: Vec<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    normals: Vec<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    uvs: Vec<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    colors: Vec<[f32; 4]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    joints: Vec<[u32; 4]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    weights: Vec<[f32; 4]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tangents: Vec<[f32; 4]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    lightmap_uvs: Vec<[f32; 2]>,
}

impl From<MeshData> for MeshDataFile {
    fn from(data: MeshData) -> Self {
        let v = data.vertices();
        let d = Vertex::default();
        // A channel's values, or nothing when every vertex holds the default there.
        fn channel<T: Copy + PartialEq>(v: &[Vertex], default: T, get: impl Fn(&Vertex) -> T) -> Vec<T> {
            if v.iter().all(|x| get(x) == default) {
                Vec::new()
            } else {
                v.iter().map(get).collect()
            }
        }
        Self {
            positions: v.iter().map(|x| x.position).collect(),
            normals: channel(v, d.normal, |x| x.normal),
            uvs: channel(v, d.tex_coords, |x| x.tex_coords),
            colors: channel(v, d.color, |x| x.color),
            joints: channel(v, d.joint_indices, |x| x.joint_indices),
            weights: channel(v, d.joint_weights, |x| x.joint_weights),
            tangents: channel(v, d.tangent, |x| x.tangent),
            lightmap_uvs: channel(v, d.lightmap_uv, |x| x.lightmap_uv),
        }
    }
}

impl TryFrom<MeshDataFile> for MeshData {
    type Error = String;

    fn try_from(file: MeshDataFile) -> Result<Self, String> {
        let n = file.positions.len();
        for (name, len) in [
            ("normals", file.normals.len()),
            ("uvs", file.uvs.len()),
            ("colors", file.colors.len()),
            ("joints", file.joints.len()),
            ("weights", file.weights.len()),
            ("tangents", file.tangents.len()),
            ("lightmap_uvs", file.lightmap_uvs.len()),
        ] {
            if len != 0 && len != n {
                return Err(format!("a MeshData's {name} hold {len} values for {n} positions"));
            }
        }
        if !n.is_multiple_of(3) {
            return Err(format!("a MeshData is a triangle list, and {n} positions are not whole triangles"));
        }
        let d = Vertex::default();
        fn pick<T: Copy>(list: &[T], i: usize, default: T) -> T {
            list.get(i).copied().unwrap_or(default)
        }
        let vertices: Vec<Vertex> = (0..n)
            .map(|i| Vertex {
                position: file.positions[i],
                normal: pick(&file.normals, i, d.normal),
                tex_coords: pick(&file.uvs, i, d.tex_coords),
                color: pick(&file.colors, i, d.color),
                joint_indices: pick(&file.joints, i, d.joint_indices),
                joint_weights: pick(&file.weights, i, d.joint_weights),
                tangent: pick(&file.tangents, i, d.tangent),
                lightmap_uv: pick(&file.lightmap_uvs, i, d.lightmap_uv),
            })
            .collect();
        Ok(Self::new(vertices))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn star() -> MeshData {
        let star: Vec<Vec2> = (0..10)
            .map(|i| {
                let a = i as f32 * std::f32::consts::TAU / 10.0;
                let r = if i % 2 == 0 { 1.0 } else { 0.45 };
                Vec2::new(a.cos() * r, a.sin() * r)
            })
            .collect();
        MeshData::extrude(&star, 0.3).expect("a star triangulates")
    }

    fn bytes(data: &MeshData) -> &[u8] {
        bytemuck::cast_slice(data.vertices())
    }

    /// **Saved and read back, a mesh is the same mesh, to the bit**: every channel of every vertex.
    /// RON writes the shortest decimal that reads back as the same `f32`, so nothing drifts.
    #[test]
    fn mesh_data_round_trips_through_ron_bit_for_bit() {
        let mut vertices = star().vertices().to_vec();
        // Every channel off its default somewhere, so each is written and read.
        vertices[0].color = [0.25, 0.5, 0.75, 1.0];
        vertices[1].joint_indices = [1, 2, 3, 4];
        vertices[1].joint_weights = [0.1, 0.2, 0.3, 0.4];
        vertices[2].tangent = [0.0, 0.0, 1.0, -1.0];
        vertices[3].position = [1.0e-7, -3.333_333_3, 1.0e9];
        vertices[4].lightmap_uv = [0.125, 0.875];
        let data = MeshData::new(vertices);
        let text = ron::to_string(&data).expect("writes");
        let back: MeshData = ron::from_str(&text).expect("reads");
        assert_eq!(bytes(&back), bytes(&data), "a vertex changed on the way through a scene file");
    }

    /// A channel every vertex holds at its default is not written: a plain extrusion is its
    /// positions, normals and UVs, and reads back with white, no skin and the default tangent.
    #[test]
    fn a_channel_at_its_default_is_left_out() {
        let plain = star();
        let text = ron::to_string(&plain).expect("writes");
        for absent in ["colors", "joints", "weights", "tangents", "lightmap_uvs"] {
            assert!(!text.contains(absent), "{absent} written for a mesh that has none: {text}");
        }
        assert!(text.contains("positions") && text.contains("normals"));
        let back: MeshData = ron::from_str(&text).expect("reads");
        assert_eq!(bytes(&back), bytes(&plain));
    }

    /// What a hand-edited or damaged file can hold is an error, not a mesh with holes in it.
    #[test]
    fn a_damaged_mesh_is_refused() {
        let two_colours = "(positions: [(0,0,0), (1,0,0), (0,1,0)], colors: [(1,1,1,1), (1,0,0,1)])";
        let error = ron::from_str::<MeshData>(two_colours).expect_err("two colours for three positions");
        assert!(error.to_string().contains("colors hold 2 values for 3 positions"), "{error}");
        let not_triangles = "(positions: [(0,0,0), (1,0,0)])";
        assert!(ron::from_str::<MeshData>(not_triangles).is_err());
    }

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
