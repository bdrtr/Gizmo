//! OBJ mesh loading (disk + async-decoded install). Extracted verbatim from `loaders.rs`.

use super::*;

impl crate::asset::AssetManager {
    /// Upload an already-decoded OBJ vertex buffer to the GPU and cache it.
    ///
    /// Called by [`AsyncAssetLoader`](crate::async_assets::AsyncAssetLoader)
    /// after decoding completes on a worker thread.
    pub fn install_obj_mesh(
        &mut self,
        device: &wgpu::Device,
        file_path: &str,
        vertices: Vec<Vertex>,
        _aabb: gizmo_math::Aabb,
    ) -> Mesh {
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("OBJ VBuf: {file_path}")),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let mesh = Mesh::new(
            device,
            Arc::new(vbuf),
            &vertices,
            Vec3::ZERO,
            format!("obj:{file_path}"),
        );
        self.mesh_cache.insert(file_path.to_string(), mesh.clone());
        mesh
    }

    /// Load an OBJ file from disk (or return the cached copy).
    pub fn load_obj(&mut self, device: &wgpu::Device, file_path_or_uuid: &str) -> Mesh {
        let file_path = match self.resolve_path_from_meta_source(file_path_or_uuid) {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("[AssetManager] ERROR: {e}");
                return self.loading_placeholder_mesh(device);
            }
        };

        // Prefer UUID as cache key when available.
        let cache_key = self
            .get_uuid(&file_path)
            .map(|id| id.to_string())
            .unwrap_or_else(|| file_path.clone());

        if let Some(cached) = self.mesh_cache.get(&cache_key) {
            return cached.clone();
        }

        let (vertices, aabb) = match decode_obj_vertices_for_async(&file_path) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("[AssetManager] OBJ load failed: {file_path} — {e}");
                // Return a valid-but-empty mesh so nothing downstream panics.
                let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Fallback VBuf (not found)"),
                    contents: &[],
                    usage: wgpu::BufferUsages::VERTEX,
                });
                return Mesh::empty(Arc::new(vbuf), format!("obj:missing_{file_path}"));
            }
        };

        self.install_obj_mesh(device, &cache_key, vertices, aabb)
    }
}
