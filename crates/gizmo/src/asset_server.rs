use crate::core::asset::Handle;
use crate::renderer::async_assets::AsyncAssetLoader;
use crate::renderer::components::{Material, Mesh};
use wgpu::util::DeviceExt;

pub struct AssetServer {
    pub loader: AsyncAssetLoader,
    mesh_paths: std::collections::HashMap<String, Handle<Mesh>>,
    _material_paths: std::collections::HashMap<String, Handle<Material>>,
}

impl Default for AssetServer {
    fn default() -> Self {
        Self::new()
    }
}

impl AssetServer {
    pub fn new() -> Self {
        Self {
            loader: AsyncAssetLoader::new(),
            mesh_paths: std::collections::HashMap::new(),
            _material_paths: std::collections::HashMap::new(),
        }
    }

    pub fn load_mesh(&mut self, path: &str) -> Handle<Mesh> {
        if let Some(handle) = self.mesh_paths.get(path) {
            return handle.clone();
        }
        let handle = Handle::new(); // Generates a new HandleId
        self.loader.request_obj_load(path.to_string(), handle.id.0);
        self.mesh_paths.insert(path.to_string(), handle.clone());
        handle
    }
}

pub fn asset_server_update_system(
    server: crate::core::system::ResMut<AssetServer>,
    renderer: crate::core::system::ResMut<crate::renderer::Renderer>,
    mut meshes: crate::core::system::ResMut<crate::core::asset::Assets<Mesh>>,
) {
    let completed = server.loader.drain_completed();

    if completed.objs.is_empty() && completed.textures.is_empty() {
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
    }
}
