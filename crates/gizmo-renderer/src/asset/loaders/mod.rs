//! glTF / OBJ asset loading for `AssetManager`, decomposed into focused submodules:
//! `obj` (OBJ meshes), `images` (texture upload + colour-space), `material` (samplers +
//! bind groups), `mesh` (vertex assembly / tangents / `parse_gltf_node`), `animation` and
//! `skeleton` (skinned-mesh data). This module keeps the public scene types and the two
//! top-level glTF entry points; each submodule owns one concern. Split from the former
//! 1347-line `loaders.rs` — pure moves, no behaviour change.

use super::decode_obj_vertices_for_async;
use super::error::AssetError;
use gizmo_animation::skeletal::{AnimationClip, Keyframe, SkeletonHierarchy, SkeletonJoint, Track};
use crate::components::{Material, Mesh};
use crate::renderer::Vertex;
use gizmo_math::{Quat, Vec3};
use std::sync::Arc;
use wgpu::util::DeviceExt;

mod animation;
mod images;
mod material;
mod mesh;
mod obj;
mod skeleton;

// Submodule helpers invoked (unqualified) by `load_gltf_from_import` below.
use animation::parse_animations;
use images::{classify_gltf_image_srgb, upload_gltf_images};
use material::build_gltf_materials;
use skeleton::parse_skeletons;

// ============================================================================
//  Public data structures
// ============================================================================

/// One node of an imported glTF scene tree, flattened into engine terms.
///
/// The transform is kept decomposed rather than as a matrix because that is how the engine's
/// `Transform` stores it, and because recomposing then re-decomposing a matrix loses the
/// distinction between a negative scale and a rotation.
pub struct GltfNodeData {
    /// Its index in the glTF document, which is how animation channels refer to it.
    pub index: usize,
    /// Its name, if the file gave it one.
    pub name: Option<String>,
    /// Index into [`GltfSceneAsset::skeletons`] if this node drives a skin.
    pub skin_index: Option<usize>,
    /// Its local translation.
    pub translation: [f32; 3],
    /// Its local rotation, as a quaternion `xyzw`.
    pub rotation: [f32; 4],
    /// Its local scale.
    pub scale: [f32; 3],
    /// (mesh, optional material) per glTF primitive on this node.
    pub primitives: Vec<(Mesh, Option<Material>)>,
    /// Its children, already resolved — the tree is materialised at import rather than left as
    /// indices.
    pub children: Vec<GltfNodeData>,
}

/// A whole imported glTF scene: its node trees, its animations and its skeletons.
pub struct GltfSceneAsset {
    /// The scene's root nodes.
    pub roots: Vec<GltfNodeData>,
    /// Every animation clip in the file.
    pub animations: Vec<AnimationClip>,
    /// Every skin, as a joint hierarchy.
    pub skeletons: Vec<SkeletonHierarchy>,
}

// ============================================================================
//  glTF — top-level entry points (orchestration; concerns live in the submodules)
// ============================================================================

impl super::AssetManager {
    /// Imports a glTF/GLB file into engine types: meshes and materials into the cache, and the
    /// node tree, animations and skeletons into the returned [`GltfSceneAsset`].
    pub fn load_gltf_scene(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture_bind_group_layout: &wgpu::BindGroupLayout,
        default_tbind: Arc<wgpu::BindGroup>,
        path_or_uuid: &str,
    ) -> Result<GltfSceneAsset, AssetError> {
        let file_path = self.resolve_path_from_meta_source(path_or_uuid)?;
        let cache_key = self
            .get_uuid(&file_path)
            .map(|id| id.to_string())
            .unwrap_or_else(|| file_path.clone());

        let import_result = if let Some(data) = self.embedded_assets.get(&file_path) {
            gltf::import_slice(data.as_ref())
        } else {
            gltf::import(&file_path)
        };

        let (document, buffers, images) =
            import_result.map_err(|source| AssetError::GltfImport {
                path: std::path::PathBuf::from(&file_path),
                source,
            })?;
        self.load_gltf_from_import(
            device,
            queue,
            texture_bind_group_layout,
            default_tbind,
            &cache_key,
            document,
            buffers,
            images,
        )
    }

    /// Upload a pre-parsed glTF import to the GPU.
    ///
    /// Split from `load_gltf_scene` so that `gltf::import` (which is
    /// CPU-bound and blocks) can be called off the main thread while GPU
    /// upload happens here on the main thread.
    pub fn load_gltf_from_import(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture_bind_group_layout: &wgpu::BindGroupLayout,
        default_tbind: Arc<wgpu::BindGroup>,
        file_path: &str,
        document: gltf::Document,
        buffers: Vec<gltf::buffer::Data>,
        images: Vec<gltf::image::Data>,
    ) -> Result<GltfSceneAsset, AssetError> {
        // ── 1. Textures ───────────────────────────────────────────────────
        // Classify each image by usage so colour maps (base/emissive) are uploaded
        // as sRGB while data maps (normal/MR/AO) stay linear, then upload.
        self.ensure_material_defaults(device, queue);
        let srgb_flags = classify_gltf_image_srgb(&document, images.len());
        let gpu_images = upload_gltf_images(device, queue, file_path, &images, &srgb_flags);

        // ── 2. Materials ──────────────────────────────────────────────────
        let defaults = self
            .material_defaults()
            .expect("material defaults ensured above");
        let gltf_materials = build_gltf_materials(
            device,
            texture_bind_group_layout,
            &document,
            &gpu_images,
            defaults,
            &default_tbind,
        );

        // ── 3. Node tree ──────────────────────────────────────────────────
        let mut roots = Vec::new();
        // One guard for the whole import, not one per root: a node named by two scenes — or
        // twice by one scene's root list — is the same duplicate as a node named by two
        // parents, and it materialises the same subtree twice. See `parse_gltf_node`.
        let mut materialised: std::collections::HashSet<usize> = std::collections::HashSet::new();
        for scene in document.scenes() {
            for node in scene.nodes() {
                if !materialised.insert(node.index()) {
                    continue;
                }
                roots.push(self.parse_gltf_node(
                    device,
                    &node,
                    &buffers,
                    &gltf_materials,
                    file_path,
                    &mut materialised,
                ));
            }
        }

        // ── 4. Animations ─────────────────────────────────────────────────
        let animations = parse_animations(&document, &buffers);

        // ── 5. Skeletons ──────────────────────────────────────────────────
        // Build a node-index → parent-node-index lookup (used when resolving
        // bone parents and the armature root transform).
        let node_parents: std::collections::HashMap<usize, usize> = document
            .nodes()
            .flat_map(|parent| {
                parent
                    .children()
                    .map(move |child| (child.index(), parent.index()))
            })
            .collect();

        // Build a fast node-index → Node lookup so we avoid O(n) `.nth()`.
        let nodes_by_index: Vec<gltf::Node> = document.nodes().collect();

        let skeletons = parse_skeletons(&document, &buffers, &node_parents, &nodes_by_index);

        Ok(GltfSceneAsset {
            roots,
            animations,
            skeletons,
        })
    }
}
