//! Asset/mesh creation convenience methods on [`Renderer`].
//!
//! These wrap [`AssetManager`](crate::asset::AssetManager) so callers can build
//! meshes/skeletons/glTF scenes directly from a `Renderer` without managing an
//! `AssetManager` themselves. Split out of `renderer.rs` for navigability —
//! no logic change.

use wgpu::util::DeviceExt;

use super::{Renderer, Vertex};

impl Renderer {
    // ==========================================================
    //  Kolaylık Metodları — Asset Oluşturma
    //  Kullanıcı `AssetManager` oluşturmak zorunda kalmadan
    //  doğrudan `renderer.create_cube()` gibi çağırabilir.
    // ==========================================================

    /// Küp mesh oluşturur.
    pub fn create_cube(&self) -> crate::components::Mesh {
        crate::asset::AssetManager::create_cube(&self.device)
    }

    /// Küre mesh oluşturur.
    pub fn create_sphere(&self, radius: f32, stacks: u32, slices: u32) -> crate::components::Mesh {
        crate::asset::AssetManager::create_sphere(&self.device, radius, stacks, slices)
    }

    /// Düzlem mesh oluşturur.
    pub fn create_plane(&self, size: f32) -> crate::components::Mesh {
        crate::asset::AssetManager::create_plane(&self.device, size)
    }

    /// Diskten bir GLTF (veya GLB) modelini senkron olarak yükler.
    pub fn load_gltf(
        &self,
        path: &str,
    ) -> Result<crate::asset::loaders::GltfSceneAsset, crate::asset::AssetError> {
        let white_tex = self.create_white_texture();
        self.asset_manager.write().unwrap().load_gltf_scene(
            &self.device,
            &self.queue,
            &self.scene.texture_bind_group_layout,
            white_tex,
            path,
        )
    }

    pub fn create_skeleton(
        &self,
        hierarchy: std::sync::Arc<gizmo_animation::skeletal::SkeletonHierarchy>,
    ) -> crate::components::Skeleton {
        // İlk local_poses'u her kemiğin orijinal local_bind_transform'undan al.
        let local_poses: Vec<gizmo_math::Mat4> = hierarchy
            .joints
            .iter()
            .map(|j| j.local_bind_transform)
            .collect();

        // Global matrislerden doğru joint_matrices hesapla (bind-pose)
        let global_matrices = hierarchy.calculate_global_matrices(&local_poses);
        let mut joint_matrices = vec![gizmo_math::Mat4::IDENTITY; 128];
        for (i, joint) in hierarchy.joints.iter().enumerate() {
            if i < 128 {
                joint_matrices[i] = global_matrices[i] * joint.inverse_bind_matrix;
            }
        }

        let buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Skeleton Joint Buffer"),
                contents: bytemuck::cast_slice(&joint_matrices),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Skeleton Bind Group"),
            layout: &self.scene.skeleton_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });

        crate::components::Skeleton::new(
            std::sync::Arc::new(bind_group),
            std::sync::Arc::new(buffer),
            hierarchy,
            local_poses,
        )
    }

    pub fn create_mesh(&self, vertices: &[Vertex]) -> wgpu::Buffer {
        self.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Mesh Vertex Buffer"),
                contents: bytemuck::cast_slice(vertices),
                usage: wgpu::BufferUsages::VERTEX,
            })
    }
}
