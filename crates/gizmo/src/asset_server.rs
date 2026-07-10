use crate::core::asset::Handle;
use crate::renderer::async_assets::AsyncAssetLoader;
use crate::renderer::components::{Material, Mesh};
use wgpu::util::DeviceExt;

pub struct AssetServer {
    pub loader: AsyncAssetLoader,
    mesh_paths: std::collections::HashMap<String, Handle<Mesh>>,
    _material_paths: std::collections::HashMap<String, Handle<Material>>,
    pub completed_gltfs: Vec<crate::renderer::async_assets::GltfImportCompletion>,
    pub completed_gltf_errors: Vec<crate::renderer::async_assets::GltfImportError>,
    /// Arka planda decode'u tamamlanan streaming texture'ları. `asset_server_update_system`
    /// bunları `drain_completed`'dan buraya biriktirir; `TextureStreamingSystem` her frame
    /// tüketip GPU'ya yükler ve ilgili entity'lerin `Material.bind_group`'unu günceller.
    /// (Eskiden `completed.textures` sessizce ATILIYORDU → streaming görsel olarak no-op'tu.)
    pub completed_textures: Vec<crate::renderer::async_assets::TextureReloadCompletion>,
    #[cfg(all(feature = "render", not(target_arch = "wasm32")))]
    pub watcher: Option<crate::renderer::hot_reload::AssetWatcher>,
}

impl Default for AssetServer {
    fn default() -> Self {
        Self::new()
    }
}

impl AssetServer {
    pub fn new() -> Self {
        #[cfg(all(feature = "render", not(target_arch = "wasm32")))]
        let watcher = crate::renderer::hot_reload::AssetWatcher::new(&["assets", "demo/assets"]);

        Self {
            loader: AsyncAssetLoader::new(),
            mesh_paths: std::collections::HashMap::new(),
            _material_paths: std::collections::HashMap::new(),
            completed_gltfs: Vec::new(),
            completed_gltf_errors: Vec::new(),
            completed_textures: Vec::new(),
            #[cfg(all(feature = "render", not(target_arch = "wasm32")))]
            watcher,
        }
    }

    pub fn load_mesh(&mut self, path: &str) -> Handle<Mesh> {
        if let Some(handle) = self.mesh_paths.get(path) {
            return handle.clone();
        }
        let handle = crate::core::asset::Handle::weak(crate::core::asset::HandleId::new());
        self.loader.request_obj_load(path.to_string(), handle.id.0);
        self.mesh_paths.insert(path.to_string(), handle.clone());
        handle
    }
}

pub fn asset_server_update_system(
    mut server: crate::core::system::ResMut<AssetServer>,
    renderer: crate::core::system::ResMut<crate::renderer::Renderer>,
    mut meshes: crate::core::system::ResMut<crate::core::asset::Assets<Mesh>>,
) {
    // Process Hot Reloading
    #[cfg(all(feature = "render", not(target_arch = "wasm32")))]
    if let Some(watcher) = &server.watcher {
        let changed = watcher.poll_changes();
        for path in changed {
            let path_str = path.to_string_lossy().to_string();
            // Check if mesh needs reloading
            if let Some(handle) = server.mesh_paths.get(&path_str) {
                tracing::info!("AssetWatcher: Reloading mesh {:?}", path_str);
                server.loader.request_obj_load(path_str.clone(), handle.id.0);
            }
        }
    }

    // Process garbage collection
    meshes.process_drops();

    let completed = server.loader.drain_completed();

    server.completed_gltfs.extend(completed.gltfs);
    server.completed_gltf_errors.extend(completed.gltf_errors);
    // Decode'u biten streaming texture'ları SAKLA (eskiden burada atılıyordu → no-op).
    // `TextureStreamingSystem` bunları GPU'ya yükleyip materyallere uygular.
    server.completed_textures.extend(completed.textures);

    if completed.objs.is_empty() {
        return;
    }

    for obj in completed.objs {
        let mesh_source = format!("obj:{}", obj.path);
        // Create wgpu buffer
        let vbuf = renderer
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(&format!("Obj VBuf: {}", obj.path)),
                contents: bytemuck::cast_slice(&obj.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let mesh = Mesh::new(
            &renderer.device,
            std::sync::Arc::new(vbuf),
            &obj.vertices,
            gizmo_math::Vec3::ZERO,
            mesh_source,
        );
        for handle_id in obj.handle_ids {
            let handle = crate::core::asset::Handle::weak(crate::core::asset::HandleId(handle_id));
            meshes.insert(&handle, mesh.clone());
        }
    }
}

pub struct AssetServerPlugin;

impl<State: 'static> crate::app::Plugin<State> for AssetServerPlugin {
    fn build(&self, app: &mut crate::app::App<State>) {
        app.world.insert_resource(AssetServer::new());
        app.schedule.add_di_system(asset_server_update_system);
        // Distance-based texture streaming: request nearby high-res textures and
        // upload+apply the ones the worker finished decoding. Runs after the drain
        // above populated `AssetServer::completed_textures` (a one-frame lag if it
        // happens to run first is harmless).
        app.schedule.add_di_system(
            gizmo_core::system::SystemConfig::new(Box::new(
                crate::systems::streaming::TextureStreamingSystem,
            ))
            .label("texture_streaming"),
        );
    }
}
